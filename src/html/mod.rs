mod parser;

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;

use anyhow::Error;
use bumpalo::collections::String as BumpString;
use bumpalo::collections::Vec as BumpVec;
use html5gum::{IoReader, Tokenizer};

use crate::paragraph::ParagraphWalker;
use crate::urls::is_external_link;

#[cfg(test)]
use pretty_assertions::assert_eq;

#[inline]
pub fn push_and_canonicalize(base: &mut BumpString, path: &str) {
    if is_external_link(path.as_bytes()) {
        base.clear();
        base.push_str(path);
        return;
    } else if path.starts_with('/') {
        base.clear();
    } else if path.is_empty() {
        if base.ends_with('/') {
            base.truncate(base.len() - 1);
        }
        return;
    } else {
        base.truncate(base.rfind('/').unwrap_or(0));
    }

    let num_slashes = path.matches('/').count();

    for (i, component) in path.split('/').enumerate() {
        match component {
            "index.html" | "index.htm" if i == num_slashes => {}
            "" | "." => {}
            ".." => {
                base.truncate(base.rfind('/').unwrap_or(0));
            }
            _ => {
                if !base.is_empty() {
                    base.push('/');
                }
                base.push_str(component);
            }
        }
    }
}

#[cfg(test)]
mod test_push_and_canonicalize {
    use super::push_and_canonicalize as push_and_canonicalize_impl;
    use super::BumpString;

    fn push_and_canonicalize(base: &mut String, path: &str) {
        let arena = bumpalo::Bump::new();
        let mut base2 = BumpString::from_str_in(&*base, &arena);
        push_and_canonicalize_impl(&mut base2, path);
        *base = base2.as_str().to_owned();
    }

    #[test]
    fn basic() {
        let mut base = String::from("2019/");
        let path = "../feed.xml";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "feed.xml");
    }

    #[test]
    fn files() {
        let mut base = String::from("contact.html");
        let path = "contact.html";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "contact.html");
    }

    #[test]
    fn empty_base() {
        let mut base = String::from("");
        let path = "./2014/article.html";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "2014/article.html");
    }

    #[test]
    fn empty_href() {
        let mut base = String::from("./foo/install.html");
        let path = "";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "./foo/install.html");

        let mut base = String::from("./foo/");
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "./foo");
    }

    #[test]
    fn index_html() {
        let mut base = String::from("foo/bar.html");
        let path = "index.html";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "foo");
    }

    #[test]
    fn index_html_middle() {
        let mut base = String::from("foo/bar.html");
        let path = "index.html/baz.html";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "foo/index.html/baz.html");
    }

    #[test]
    fn external_scheme_index() {
        let mut base = String::from("index.html");
        let path = "http://foo.com";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "http://foo.com");
    }

    #[test]
    fn external_scheme_empty_base() {
        let mut base = String::from("");
        let path = "http://foo.com";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "http://foo.com");
    }

    #[test]
    fn external_scheme_relative() {
        let mut base = String::from("bar.html");
        let path = "//foo.com";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "//foo.com");
    }

    #[test]
    fn external_scheme_subdir() {
        let mut base = String::from("foo/bar.html");
        let path = "http://foo.com";
        push_and_canonicalize(&mut base, path);
        assert_eq!(base, "http://foo.com");
    }
}

#[inline]
pub fn try_percent_decode(input: &str) -> Cow<'_, str> {
    percent_encoding::percent_decode_str(input)
        .decode_utf8()
        .unwrap_or(Cow::Borrowed(input))
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Href<'a>(pub &'a str);

impl<'a> Href<'a> {
    pub fn without_anchor(&self) -> Href<'_> {
        let mut s = self.0;

        if let Some(i) = s.find('#') {
            s = &s[..i];
        }

        Href(s)
    }
}

impl<'a> fmt::Display for Href<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct UsedLink<'a, P> {
    pub href: Href<'a>,
    pub path: Arc<PathBuf>,
    pub paragraph: Option<P>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct DefinedLink<'a> {
    pub href: Href<'a>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum Link<'a, P> {
    Uses(UsedLink<'a, P>),
    Defines(DefinedLink<'a>),
}

impl<'a, P> Link<'a, P> {
    pub fn into_paragraph(self) -> Option<P> {
        match self {
            Link::Uses(UsedLink { paragraph, .. }) => paragraph,
            Link::Defines(_) => None,
        }
    }
}

const BUF_SIZE: usize = 1024 * 1024;

/// This struct is initialized once per "batch of documents" that will be processed on a single
/// worker thread (as determined by rayon). It pays off to do as much heap allocation as possible
/// here once instead of in Document::links.
pub struct DocumentBuffers {
    arena: bumpalo::Bump,
    html_read_buffer: Box<[u8; BUF_SIZE]>,
    parser_buffers: parser::ParserBuffers,
}

impl Default for DocumentBuffers {
    fn default() -> Self {
        DocumentBuffers {
            arena: Default::default(),
            html_read_buffer: Box::new([0; BUF_SIZE]),
            parser_buffers: Default::default(),
        }
    }
}

impl DocumentBuffers {
    pub fn reset(&mut self) {
        self.arena.reset();
        self.parser_buffers.reset();
    }
}

pub struct Document {
    pub path: Arc<PathBuf>,
    href: String,
    pub is_index_html: bool,
}

impl Document {
    pub fn new(base_path: &Path, path: &Path) -> Self {
        let mut href_path = path
            .strip_prefix(base_path)
            .expect("base_path is not a base of path");

        let is_index_html = href_path.ends_with("index.html") || href_path.ends_with("index.htm");

        if is_index_html {
            href_path = href_path.parent().unwrap_or(href_path);
        }

        let mut href = href_path
            .to_str()
            .expect("Invalid unicode in path")
            .to_owned();

        if cfg!(windows) {
            unsafe {
                // safety: we replace ascii bytes only
                // safety: href is an exclusive reference or owned string
                let href = href.as_bytes_mut();
                for b in href.iter_mut() {
                    if *b == b'\\' {
                        *b = b'/';
                    }
                }
            }
        }

        Document {
            path: Arc::new(path.to_owned()),
            href,
            is_index_html,
        }
    }

    pub fn href(&self) -> Href<'_> {
        Href(&self.href)
    }

    fn join<'b>(
        &self,
        arena: &'b bumpalo::Bump,
        preserve_anchor: bool,
        rel_href: &str,
    ) -> Href<'b> {
        let qs_start = rel_href.find(&['?', '#'][..]).unwrap_or(rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or(rel_href.len());

        let mut href = BumpString::from_str_in(&self.href, arena);
        if self.is_index_html {
            href.push('/');
        }

        push_and_canonicalize(&mut href, &try_percent_decode(&rel_href[..qs_start]));

        if preserve_anchor {
            let anchor = &rel_href[anchor_start..];
            if anchor.len() > 1 {
                href.push_str(&try_percent_decode(anchor));
            }
        }

        Href(href.into_bump_str())
    }

    pub fn links<'b, 'l, P: ParagraphWalker>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        check_anchors: bool,
    ) -> Result<impl Iterator<Item = Link<'l, P::Paragraph>>, Error>
    where
        'b: 'l,
    {
        self.links_from_read::<_, P>(doc_buf, fs::File::open(&*self.path)?, check_anchors)
    }

    fn links_from_read<'b, 'l, R: Read, P: ParagraphWalker>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        read: R,
        check_anchors: bool,
    ) -> Result<impl Iterator<Item = Link<'l, P::Paragraph>>, Error>
    where
        'b: 'l,
    {
        let mut link_buf = BumpVec::new_in(&doc_buf.arena);

        {
            let emitter = parser::HyperlinkEmitter {
                paragraph_walker: P::new(),
                arena: &doc_buf.arena,
                document: self,
                link_buf: &mut link_buf,
                in_paragraph: false,
                last_paragraph_i: 0,
                buffers: &mut doc_buf.parser_buffers,
                current_tag_is_closing: false,
                check_anchors,
            };
            let ioreader = IoReader::new_with_buffer(read, doc_buf.html_read_buffer.as_mut());
            let reader = Tokenizer::new_with_emitter(ioreader, emitter);

            for error in reader {
                error?;
            }
        }

        Ok(link_buf.into_iter())
    }
}

#[test]
fn test_document_href() {
    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting/index.html"),
    );

    assert_eq!(doc.href(), Href("platforms/python/troubleshooting"));

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting.html"),
    );

    assert_eq!(doc.href(), Href("platforms/python/troubleshooting.html"));
}

#[test]
fn test_html_parsing_malformed_script() {
    use crate::paragraph::ParagraphHasher;

    let html = r###"
        <a href=foo />
        <script>
        ...
         * @typedef {{
         *     name: string,
         *     id: string,
         *     score: number,
         *     description: string,
         *     audits: !Array<!ReportRenderer.AuditJSON>
         * }}
        <a href=wut />
        ...
        </script>
        <a href=bar />
    "###;

    let doc = Document::new(Path::new("public/"), Path::new("public/hello.html"));

    let mut doc_buf = DocumentBuffers::default();

    let links = doc
        .links_from_read::<_, ParagraphHasher>(&mut doc_buf, html.as_bytes(), false)
        .unwrap();

    let used_link = |x: &'static str| {
        Link::Uses(UsedLink {
            href: Href(x),
            path: doc.path.clone(),
            paragraph: None,
        })
    };

    assert_eq!(
        links.collect::<Vec<_>>(),
        &[used_link("foo"), used_link("bar")]
    );
}

#[test]
fn test_document_links() {
    use bumpalo::Bump;

    use crate::collector::canonicalize_local_link;
    use crate::paragraph::ParagraphHasher;

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting/index.html"),
    );

    let mut doc_buf = DocumentBuffers::default();

    let links = doc.links_from_read::<_, ParagraphHasher>(
        &mut doc_buf,
        r#"""
        <!doctype html>
        &nbsp;
    <a href="../../ruby/" />
    <a href="/platforms/perl/">Perl</a>

    <a href=../../rust/>
    <a href='../../go/?foo=bar&bar=baz' href='../../go/'>
    <a href="&#109;&#97;" />

    <!-- test url encoding within HTML + percent-encoding within html encoding -->
    <a href="%5Bslug%5D.js" />
    <a href="&#37;5Bschlug%5D.js" />

    <!-- obfuscated mailto: link -->
    <a href='&#109;&#97;&#105;&#108;&#116;&#111;&#58;&#102;&#111;&#111;&#64;&#101;&#120;&#97;&#109;&#112;&#108;&#101;&#46;&#99;&#111;&#109;' />

    <!-- case sensitivity -->
    <A HREF='case' />
    <A HREF='HTTP://googel.com' />

    <!-- whitespace, this is how browsers really do behave -->
    <a href=' whitespace ' />

    <img
        src="/static/image.png"
        srcset="
        /static/image300.png  300w,
        /static/image600.png  600w,
        "
    />
    """#
        .as_bytes(),
        false,
    )
    .unwrap();

    let used_link = |x: &'static str| {
        Link::Uses(UsedLink {
            href: Href(x),
            path: doc.path.clone(),
            paragraph: None,
        })
    };

    let arena = Bump::new();

    assert_eq!(
        &links
            .filter_map(|x| canonicalize_local_link(&arena, x))
            .collect::<Vec<_>>(),
        &[
            used_link("platforms/ruby"),
            used_link("platforms/perl"),
            used_link("platforms/rust"),
            used_link("platforms/go"),
            used_link("platforms/go"),
            used_link("platforms/python/troubleshooting/ma"),
            used_link("platforms/python/troubleshooting/[slug].js"),
            used_link("platforms/python/troubleshooting/[schlug].js"),
            used_link("platforms/python/troubleshooting/case"),
            used_link("platforms/python/troubleshooting/whitespace"),
            used_link("static/image.png"),
            used_link("static/image300.png"),
            used_link("static/image600.png"),
        ]
    );
}

#[test]
fn test_document_join_index_html() {
    let arena = bumpalo::Bump::new();

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting/index.html"),
    );

    assert_eq!(
        doc.join(&arena, false, "../../ruby#foo"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, true, "../../ruby#foo"),
        Href("platforms/ruby#foo")
    );
    assert_eq!(
        doc.join(&arena, true, "../../ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );

    assert_eq!(
        doc.join(&arena, false, "/platforms/ruby"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );
}

#[test]
fn test_document_join_bare_html() {
    let arena = bumpalo::Bump::new();

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting.html"),
    );

    assert_eq!(
        doc.join(&arena, false, "../ruby#foo"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, true, "../ruby#foo"),
        Href("platforms/ruby#foo")
    );
    assert_eq!(
        doc.join(&arena, true, "../ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );

    assert_eq!(
        doc.join(&arena, false, "/platforms/ruby"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );
    assert_eq!(
        doc.join(&arena, false, "/locations/troms%C3%B8"),
        Href("locations/tromsø")
    );
    assert_eq!(
        doc.join(&arena, true, "/locations/oslo#gr%C3%BCnerl%C3%B8kka"),
        Href("locations/oslo#grünerløkka")
    );
}

#[test]
fn test_json_script() {
    use crate::paragraph::ParagraphHasher;

    let doc = Document::new(Path::new("/"), Path::new("/html5gum/struct.Tokenizer.html"));

    let html = r#"<script type="text/json" id="notable-traits-data">{"InfallibleTokenizer<R, E>":"<h3>Notable traits for <code><a class=\"struct\" href=\"struct.InfallibleTokenizer.html\" title=\"struct html5gum::InfallibleTokenizer\">InfallibleTokenizer</a>&lt;R, E&gt;</code></h3><pre><code><div class=\"where\">impl&lt;R: <a class=\"trait\" href=\"trait.Reader.html\" title=\"trait html5gum::Reader\">Reader</a>&lt;Error = <a class=\"enum\" href=\"https://doc.rust-lang.org/1.82.0/core/convert/enum.Infallible.html\" title=\"enum core::convert::Infallible\">Infallible</a>&gt;, E: <a class=\"trait\" href=\"emitters/trait.Emitter.html\" title=\"trait html5gum::emitters::Emitter\">Emitter</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.82.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a> for <a class=\"struct\" href=\"struct.InfallibleTokenizer.html\" title=\"struct html5gum::InfallibleTokenizer\">InfallibleTokenizer</a>&lt;R, E&gt;</div><div class=\"where\">    type <a href=\"https://doc.rust-lang.org/1.82.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" class=\"associatedtype\">Item</a> = E::<a class=\"associatedtype\" href=\"emitters/trait.Emitter.html#associatedtype.Token\" title=\"type html5gum::emitters::Emitter::Token\">Token</a>;</div>"}</script>"#;

    let mut doc_buf = DocumentBuffers::default();

    let links = doc
        .links_from_read::<_, ParagraphHasher>(&mut doc_buf, html.as_bytes(), false)
        .unwrap();

    assert_eq!(links.collect::<Vec<_>>(), &[]);
}
