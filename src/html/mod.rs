mod parser;

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;

use anyhow::Error;
use html5gum::{IoReader, Tokenizer};

use crate::paragraph::{ParagraphWalker, VoidParagraph};
use crate::urls::is_external_link;

#[cfg(test)]
use pretty_assertions::assert_eq;

fn is_dynamic_redirect(path: &str) -> bool {
    path.contains("/:") || path.contains("/@") || path.contains('*')
}

#[inline]
pub fn push_and_canonicalize(base: &mut String, path: &str) {
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

    let mut components = path.split('/').peekable();

    while let Some(component) = components.next() {
        let is_last = components.peek().is_none();
        match component {
            "index.html" | "index.htm" if is_last => {}
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
    use super::push_and_canonicalize;

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

impl Href<'_> {
    pub fn without_anchor(&self) -> Href<'_> {
        let mut s = self.0;

        if let Some(i) = s.find('#') {
            s = &s[..i];
        }

        Href(s)
    }
}

impl fmt::Display for Href<'_> {
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

    /// Replace the paragraph of this link, converting between paragraph types.
    fn with_paragraph<Q>(self, paragraph: Option<Q>) -> Link<'a, Q> {
        match self {
            Link::Uses(UsedLink { href, path, .. }) => Link::Uses(UsedLink {
                href,
                path,
                paragraph,
            }),
            Link::Defines(DefinedLink { href }) => Link::Defines(DefinedLink { href }),
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
    /// Scratch space for building hrefs in. The final string is copied into the arena with
    /// alloc_str, so the arena never sees any grow-and-copy garbage.
    href_buf: String,
    /// Scratch space for the parser's paragraph link buffer. Outside of links_from_read this is
    /// always empty, only the allocation is reused across documents (the 'static lifetime is
    /// laundered through recycle()). Buffered links never carry a paragraph -- it is attached
    /// when the buffer is flushed -- hence VoidParagraph.
    link_buf: Vec<Link<'static, VoidParagraph>>,
}

impl Default for DocumentBuffers {
    fn default() -> Self {
        DocumentBuffers {
            arena: Default::default(),
            html_read_buffer: Box::new([0; BUF_SIZE]),
            parser_buffers: Default::default(),
            href_buf: String::new(),
            link_buf: Vec::new(),
        }
    }
}

impl DocumentBuffers {
    pub fn reset(&mut self) {
        self.arena.reset();
        self.parser_buffers.reset();
        self.href_buf.clear();
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
        scratch: &mut String,
        preserve_anchor: bool,
        rel_href: &str,
    ) -> Href<'b> {
        let qs_start = rel_href.find(&['?', '#'][..]).unwrap_or(rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or(rel_href.len());

        scratch.clear();
        scratch.push_str(&self.href);
        if self.is_index_html {
            scratch.push('/');
        }

        push_and_canonicalize(scratch, &try_percent_decode(&rel_href[..qs_start]));

        if preserve_anchor {
            let anchor = &rel_href[anchor_start..];
            if anchor.len() > 1 {
                scratch.push_str(&try_percent_decode(anchor));
            }
        }

        Href(arena.alloc_str(scratch))
    }

    /// Construct the href under which an anchor (e.g. `id="foo"`) in this document is reachable.
    ///
    /// Equivalent to `self.join(arena, scratch, true, &format!("#{anchor}"))`, but without
    /// having to build the intermediate `#`-prefixed string (which would alias `scratch`).
    fn join_anchor<'b>(
        &self,
        arena: &'b bumpalo::Bump,
        scratch: &mut String,
        anchor: &str,
    ) -> Href<'b> {
        scratch.clear();
        scratch.push_str(&self.href);

        if !anchor.is_empty() {
            scratch.push('#');
            scratch.push_str(&try_percent_decode(anchor));
        }

        Href(arena.alloc_str(scratch))
    }

    pub fn extract_links<'b, 'l, P: ParagraphWalker, F>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        check_anchors: bool,
        callback: F,
    ) -> Result<bool, Error>
    where
        'b: 'l,
        F: FnMut(Link<'l, P::Paragraph>),
    {
        if self.href == "_redirects" {
            self.parse_redirects::<P, F>(doc_buf, check_anchors, callback)?;
            return Ok(true);
        }

        if self
            .path
            .extension()
            .and_then(|extension| {
                let ext = extension.to_str()?;
                Some(ext == "html" || ext == "htm")
            })
            .unwrap_or(false)
        {
            self.links::<P, F>(doc_buf, check_anchors, callback)?;
            return Ok(true);
        }

        Ok(false)
    }

    pub fn links<'b, 'l, P: ParagraphWalker, F>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        check_anchors: bool,
        callback: F,
    ) -> Result<(), Error>
    where
        'b: 'l,
        F: FnMut(Link<'l, P::Paragraph>),
    {
        self.links_from_read::<_, P, F>(
            doc_buf,
            fs::File::open(&*self.path)?,
            check_anchors,
            callback,
        )
    }

    fn parse_redirects<'b, 'l, P: ParagraphWalker, F>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        check_anchors: bool,
        callback: F,
    ) -> Result<(), Error>
    where
        'b: 'l,
        F: FnMut(Link<'l, P::Paragraph>),
    {
        self.redirects_from_read::<_, P, F>(
            doc_buf,
            BufReader::new(fs::File::open(&*self.path)?),
            check_anchors,
            callback,
        )
    }

    fn redirects_from_read<'b, 'l, R: BufRead, P: ParagraphWalker, F>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        reader: R,
        check_anchors: bool,
        mut callback: F,
    ) -> Result<(), Error>
    where
        'b: 'l,
        F: FnMut(Link<'l, P::Paragraph>),
    {
        for line in reader.lines() {
            let line = line?;

            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let source = parts[0];
                let target = parts[1];

                // Skip dynamic redirects with placeholders (:param, @param, or * splat)
                if is_dynamic_redirect(source) || is_dynamic_redirect(target) {
                    continue;
                }

                callback(Link::Defines(DefinedLink {
                    href: self.join(&doc_buf.arena, &mut doc_buf.href_buf, check_anchors, source),
                }));

                if !is_external_link(target.as_bytes()) {
                    callback(Link::Uses(UsedLink {
                        href: self.join(
                            &doc_buf.arena,
                            &mut doc_buf.href_buf,
                            check_anchors,
                            target,
                        ),
                        path: self.path.clone(),
                        paragraph: None,
                    }));
                }
            }
        }

        Ok(())
    }

    fn links_from_read<'b, 'l, R: Read, P: ParagraphWalker, F>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        read: R,
        check_anchors: bool,
        callback: F,
    ) -> Result<(), Error>
    where
        'b: 'l,
        F: FnMut(Link<'l, P::Paragraph>),
    {
        // borrow the recycled link buffer from doc_buf, shortening 'static to the arena borrow
        // (safe, Vec and Link are covariant). On the error path below, the buffer is dropped and
        // its allocation is simply not reused.
        let mut link_buf: Vec<Link<'_, VoidParagraph>> = std::mem::take(&mut doc_buf.link_buf);

        let emitter = parser::HyperlinkEmitter::<P, _>::new(
            &doc_buf.arena,
            self,
            &mut doc_buf.parser_buffers,
            &mut doc_buf.href_buf,
            &mut link_buf,
            check_anchors,
            callback,
        );
        let ioreader = IoReader::new_with_buffer(read, doc_buf.html_read_buffer.as_mut());
        let reader = Tokenizer::new_with_emitter(ioreader, emitter);

        for error in reader {
            error?;
        }

        doc_buf.link_buf = recycle_vec::VecExt::recycle(link_buf);

        Ok(())
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

    let mut links = Vec::new();
    doc.links_from_read::<_, ParagraphHasher, _>(&mut doc_buf, html.as_bytes(), false, |link| {
        links.push(link)
    })
    .unwrap();

    let used_link = |x: &'static str| {
        Link::Uses(UsedLink {
            href: Href(x),
            path: doc.path.clone(),
            paragraph: None,
        })
    };

    assert_eq!(links, &[used_link("foo"), used_link("bar")]);
}

#[test]
fn test_document_links() {
    use crate::collector::filter_local_link;
    use crate::paragraph::ParagraphHasher;

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting/index.html"),
    );

    let mut doc_buf = DocumentBuffers::default();

    let mut links = Vec::new();

    doc.links_from_read::<_, ParagraphHasher, _>(
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
        |link| links.extend(filter_local_link(link)),
    )
    .unwrap();

    let used_link = |x: &'static str| {
        Link::Uses(UsedLink {
            href: Href(x),
            path: doc.path.clone(),
            paragraph: None,
        })
    };

    assert_eq!(
        &links,
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
    let mut scratch = String::new();

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting/index.html"),
    );

    assert_eq!(
        doc.join(&arena, &mut scratch, false, "../../ruby#foo"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, true, "../../ruby#foo"),
        Href("platforms/ruby#foo")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, true, "../../ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );

    assert_eq!(
        doc.join(&arena, &mut scratch, false, "/platforms/ruby"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );
}

#[test]
fn test_document_join_bare_html() {
    let arena = bumpalo::Bump::new();
    let mut scratch = String::new();

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting.html"),
    );

    assert_eq!(
        doc.join(&arena, &mut scratch, false, "../ruby#foo"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, true, "../ruby#foo"),
        Href("platforms/ruby#foo")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, true, "../ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );

    assert_eq!(
        doc.join(&arena, &mut scratch, false, "/platforms/ruby"),
        Href("platforms/ruby")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo")
    );
    assert_eq!(
        doc.join(&arena, &mut scratch, false, "/locations/troms%C3%B8"),
        Href("locations/tromsø")
    );
    assert_eq!(
        doc.join(
            &arena,
            &mut scratch,
            true,
            "/locations/oslo#gr%C3%BCnerl%C3%B8kka"
        ),
        Href("locations/oslo#grünerløkka")
    );
}

#[test]
fn test_paragraph_nesting() {
    use crate::paragraph::{DebugParagraphWalker, ParagraphHasher};

    // li and p are both paragraph tags. html5gum is not a tree builder, so the opening p does
    // not implicitly close the li: it ends the li's paragraph, and links buffered under the li
    // get the paragraph text accumulated up to that point. links outside of any paragraph
    // get no paragraph at all, while links in a text-less paragraph (empty) get an empty one.
    let html = r#"
        <a href=first />
        <p><a href=empty /></p>
        <a href=between />
        <li>
            one two
            <a href=before />
            <p>three four <a href=inside /></p>
            <a href=after />
        </li>
    "#;

    let doc = Document::new(Path::new("public/"), Path::new("public/hello.html"));

    let mut doc_buf = DocumentBuffers::default();

    let mut links = Vec::new();
    doc.links_from_read::<_, DebugParagraphWalker<ParagraphHasher>, _>(
        &mut doc_buf,
        html.as_bytes(),
        false,
        |link| links.push(link),
    )
    .unwrap();

    let links: Vec<_> = links
        .into_iter()
        .map(|link| match link {
            Link::Uses(used_link) => (used_link.href.0, used_link.paragraph.map(|p| p.to_string())),
            Link::Defines(_) => panic!("unexpected defined link"),
        })
        .collect();

    assert_eq!(
        links,
        &[
            ("first", None),
            ("empty", Some("".to_string())),
            ("between", None),
            ("before", Some("onetwo".to_string())),
            ("inside", Some("threefour".to_string())),
            ("after", None),
        ]
    );
}

#[test]
fn test_json_script() {
    use crate::paragraph::ParagraphHasher;

    let doc = Document::new(Path::new("/"), Path::new("/html5gum/struct.Tokenizer.html"));

    let html = r#"<script type="text/json" id="notable-traits-data">{"InfallibleTokenizer<R, E>":"<h3>Notable traits for <code><a class=\"struct\" href=\"struct.InfallibleTokenizer.html\" title=\"struct html5gum::InfallibleTokenizer\">InfallibleTokenizer</a>&lt;R, E&gt;</code></h3><pre><code><div class=\"where\">impl&lt;R: <a class=\"trait\" href=\"trait.Reader.html\" title=\"trait html5gum::Reader\">Reader</a>&lt;Error = <a class=\"enum\" href=\"https://doc.rust-lang.org/1.82.0/core/convert/enum.Infallible.html\" title=\"enum core::convert::Infallible\">Infallible</a>&gt;, E: <a class=\"trait\" href=\"emitters/trait.Emitter.html\" title=\"trait html5gum::emitters::Emitter\">Emitter</a>&gt; <a class=\"trait\" href=\"https://doc.rust-lang.org/1.82.0/core/iter/traits/iterator/trait.Iterator.html\" title=\"trait core::iter::traits::iterator::Iterator\">Iterator</a> for <a class=\"struct\" href=\"struct.InfallibleTokenizer.html\" title=\"struct html5gum::InfallibleTokenizer\">InfallibleTokenizer</a>&lt;R, E&gt;</div><div class=\"where\">    type <a href=\"https://doc.rust-lang.org/1.82.0/core/iter/traits/iterator/trait.Iterator.html#associatedtype.Item\" class=\"associatedtype\">Item</a> = E::<a class=\"associatedtype\" href=\"emitters/trait.Emitter.html#associatedtype.Token\" title=\"type html5gum::emitters::Emitter::Token\">Token</a>;</div>"}</script>"#;

    let mut doc_buf = DocumentBuffers::default();

    let mut links = Vec::new();
    doc.links_from_read::<_, ParagraphHasher, _>(&mut doc_buf, html.as_bytes(), false, |link| {
        links.push(link)
    })
    .unwrap();

    assert_eq!(links, &[]);
}

#[test]
fn test_redirects_dynamic_placeholders() {
    use crate::paragraph::NoopParagraphWalker;

    let doc = Document::new(Path::new("public/"), Path::new("public/_redirects"));

    let redirects = "\
        # Dynamic redirects should be skipped\n\
        /roster/:slug/matches /roster/:slug/matches/1\n\
        /roster/@slug/matches /roster/@slug/matches/1\n\
        /docs/* /new-docs/:splat\n\
        /static /target.html\n";

    let mut doc_buf = DocumentBuffers::default();
    let mut links = Vec::new();
    doc.redirects_from_read::<_, NoopParagraphWalker, _>(
        &mut doc_buf,
        redirects.as_bytes(),
        false,
        |link| links.push(link),
    )
    .unwrap();

    assert_eq!(links.len(), 2);
    assert!(matches!(&links[0], Link::Defines(DefinedLink { href }) if href.0 == "static"));
    assert!(matches!(&links[1], Link::Uses(UsedLink { href, .. }) if href.0 == "target.html"));
}
