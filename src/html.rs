use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;

use anyhow::Error;
use bumpalo::collections::vec::Vec as BumpVec;
use bumpalo::collections::String as BumpString;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::paragraph::ParagraphWalker;

#[cfg(test)]
use pretty_assertions::assert_eq;

#[inline]
fn is_paragraph_tag(tag: &[u8]) -> bool {
    tag == b"p" || tag == b"li" || tag == b"dt" || tag == b"dd"
}

#[inline]
fn is_bad_schema(url: &[u8]) -> bool {
    // check if url is empty
    let first_char = match url.first() {
        Some(x) => x,
        None => return false,
    };

    // protocol-relative URL
    if url.starts_with(b"//") {
        return true;
    }

    // check if string before first : is a valid URL scheme
    // see RFC 2396, Appendix A for what constitutes a valid scheme

    if !matches!(first_char, b'a'..=b'z' | b'A'..=b'Z') {
        return false;
    }

    for c in &url[1..] {
        match c {
            b'a'..=b'z' => (),
            b'A'..=b'Z' => (),
            b'0'..=b'9' => (),
            b'+' => (),
            b'-' => (),
            b'.' => (),
            b':' => return true,
            _ => return false,
        }
    }

    false
}

#[inline]
fn push_and_canonicalize(base: &mut BumpString, path: &str) {
    if path.starts_with('/') {
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
}

#[inline]
fn try_percent_decode(input: &str) -> Cow<'_, str> {
    percent_encoding::percent_decode_str(input)
        .decode_utf8()
        .unwrap_or(Cow::Borrowed(input))
}

#[inline]
pub fn trim_ascii_whitespace(x: Cow<'_, [u8]>) -> Cow<'_, [u8]> {
    let from = match x.iter().position(|x| !x.is_ascii_whitespace()) {
        Some(i) => i,
        None => return x,
    };
    let to = x.iter().rposition(|x| !x.is_ascii_whitespace()).unwrap();
    Cow::Owned(x[from..=to].to_owned())
}

#[inline]
fn try_unescape_attribute_value<'a>(attr: &'a Attribute<'_>) -> Cow<'a, [u8]> {
    // decode html and trim ascii whitespace
    // XXX: this is only necessary because quick-xml is not a proper html parser
    trim_ascii_whitespace(attr.unescaped_value().unwrap_or(Cow::Borrowed(&attr.value)))
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

/// This struct is initialized once per "batch of documents" that will be processed on a single
/// worker thread (as determined by rayon). It pays off to do as much heap allocation as possible
/// here once instead of in Document::links.
#[derive(Default)]
pub struct DocumentBuffers {
    arena: bumpalo::Bump,
    // quick-xml requires a std vec
    xml_buf: Vec<u8>,
}

impl DocumentBuffers {
    pub fn reset(&mut self) {
        self.arena.reset();
        self.xml_buf.clear();
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
        let qs_start = rel_href
            .find(&['?', '#'][..])
            .unwrap_or_else(|| rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or_else(|| rel_href.len());

        let mut href = BumpString::from_str_in(&self.href, arena);
        if self.is_index_html {
            href.push('/');
        }

        push_and_canonicalize(&mut href, &try_percent_decode(&rel_href[..qs_start]));

        if preserve_anchor {
            let anchor = &rel_href[anchor_start..];
            if anchor.len() > 1 {
                href.push_str(anchor);
            }
        }

        Href(href.into_bump_str())
    }

    pub fn links<'b, 'l, P: ParagraphWalker>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        check_anchors: bool,
        get_paragraphs: bool,
    ) -> Result<impl Iterator<Item = Link<'l, P::Paragraph>>, Error>
    where
        'b: 'l,
    {
        self.links_from_read::<_, P>(
            doc_buf,
            fs::File::open(&*self.path)?,
            check_anchors,
            get_paragraphs,
        )
    }

    fn links_from_read<'b, 'l, R: Read, P: ParagraphWalker>(
        &self,
        doc_buf: &'b mut DocumentBuffers,
        read: R,
        check_anchors: bool,
        get_paragraphs: bool,
    ) -> Result<impl Iterator<Item = Link<'l, P::Paragraph>>, Error>
    where
        'b: 'l,
    {
        let mut reader = Reader::from_reader(BufReader::new(read));
        reader.trim_text(true);
        reader.expand_empty_elements(true);
        reader.check_end_names(false);

        let mut paragraph_walker = P::new();
        let mut link_buf = BumpVec::new_in(&doc_buf.arena);
        let mut last_paragraph_i = 0;
        let mut in_paragraph = false;

        loop {
            match reader.read_event(&mut doc_buf.xml_buf)? {
                Event::Eof => break,
                Event::Start(ref e) => {
                    if is_paragraph_tag(e.name()) {
                        in_paragraph = true;
                        last_paragraph_i = link_buf.len();
                        paragraph_walker.finish_paragraph();
                    }

                    macro_rules! extract_used_link {
                        ($attr_name:expr) => {
                            for attr in e.html_attributes().with_checks(false) {
                                let attr = attr?;

                                if !attr.key.eq_ignore_ascii_case($attr_name) {
                                    continue;
                                }

                                let value = try_unescape_attribute_value(&attr);

                                if is_bad_schema(&value) {
                                    continue;
                                }

                                link_buf.push(Link::Uses(UsedLink {
                                    href: self.join(
                                        &doc_buf.arena,
                                        check_anchors,
                                        str::from_utf8(&value)?,
                                    ),
                                    path: self.path.clone(),
                                    paragraph: None,
                                }));
                            }
                        };
                    }

                    macro_rules! extract_used_link_srcset {
                        ($attr_name:expr) => {
                            for attr in e.html_attributes().with_checks(false) {
                                let attr = attr?;

                                if !attr.key.eq_ignore_ascii_case($attr_name) {
                                    continue;
                                }

                                let values = try_unescape_attribute_value(&attr);

                                // https://html.spec.whatwg.org/multipage/images.html#srcset-attribute
                                for value in values.split(|&c| c == b',')
                                    .filter_map(|image_candidate_string| image_candidate_string.split(|&c| c == b' ').filter(|value| !value.is_empty()).next())
                                    .filter(|value| !value.is_empty()) {
                                    if is_bad_schema(&value) {
                                        continue;
                                    }

                                    link_buf.push(Link::Uses(UsedLink {
                                        href: self.join(
                                            &doc_buf.arena,
                                            check_anchors,
                                            str::from_utf8(&value)?,
                                        ),
                                        path: self.path.clone(),
                                        paragraph: None,
                                    }));
                                }
                            }
                        }
                    }

                    macro_rules! extract_anchor_def {
                        ($attr_name:expr) => {
                            if check_anchors {
                                for attr in e.html_attributes().with_checks(false) {
                                    let attr = attr?;

                                    if attr.key.eq_ignore_ascii_case($attr_name) {
                                        let mut href = BumpString::new_in(&doc_buf.arena);

                                        let value = try_unescape_attribute_value(&attr);

                                        href.push('#');
                                        href.push_str(str::from_utf8(&value)?);

                                        link_buf.push(Link::Defines(DefinedLink {
                                            href: self.join(&doc_buf.arena, check_anchors, &href),
                                        }));
                                    }
                                }
                            }
                        };
                    }

                    // XXX: Those macros are not a great way to organize code units. If you're
                    // considering refactoring them, don't bother with closures.
                    //
                    // * Rust will complain that the closures all borrow link_buf mutably which
                    //   makes it impossible to access immutably outside of the closure.
                    // * Rust will complain that the closures may outlive the current function.
                    //
                    // In theory, when Rust generates the struct for the closure to capture local
                    // variable state, that struct would have to be self-referential.
                    //
                    // In the end you're forced to pass almost all arguments with lifetime
                    // constraints explicitly, so you might as well use a function on `self`.
                    //
                    // There's also a performance optimization left on the table because we iterate
                    // through the element's attributes multiple times, once per macro call.

                    if e.name().eq_ignore_ascii_case(b"a") {
                        extract_used_link!(b"href");
                        extract_anchor_def!(b"name");
                    } else if e.name().eq_ignore_ascii_case(b"img") {
                        extract_used_link!(b"src");
                        extract_used_link_srcset!(b"srcSet");
                    } else if e.name().eq_ignore_ascii_case(b"link") {
                        extract_used_link!(b"href");
                    } else if e.name().eq_ignore_ascii_case(b"script")
                        || e.name().eq_ignore_ascii_case(b"iframe")
                    {
                        extract_used_link!(b"src");
                    } else if e.name().eq_ignore_ascii_case(b"area") {
                        extract_used_link!(b"href");
                    } else if e.name().eq_ignore_ascii_case(b"object") {
                        extract_used_link!(b"data");
                    }

                    extract_anchor_def!(b"id");
                }
                Event::End(e) if get_paragraphs => {
                    if is_paragraph_tag(e.name()) {
                        let paragraph = paragraph_walker.finish_paragraph();
                        if in_paragraph {
                            for link in &mut link_buf[last_paragraph_i..] {
                                match link {
                                    Link::Uses(ref mut x) => {
                                        x.paragraph = paragraph.clone();
                                    }
                                    Link::Defines(_) => {}
                                }
                            }
                            in_paragraph = false;
                        }
                        last_paragraph_i = link_buf.len();
                    }
                }
                Event::Text(e) if get_paragraphs && in_paragraph => {
                    let text = e.unescaped().unwrap_or_else(|_| e.escaped().into());
                    paragraph_walker.update(&text);
                }
                _ => {}
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

    assert_eq!(doc.href(), Href("platforms/python/troubleshooting".into()));

    let doc = Document::new(
        Path::new("public/"),
        Path::new("public/platforms/python/troubleshooting.html"),
    );

    assert_eq!(
        doc.href(),
        Href("platforms/python/troubleshooting.html".into())
    );
}

#[test]
fn test_document_links() {
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
    """#
        .as_bytes(),
        false,
        false,
    )
    .unwrap();

    let used_link = |x: &'static str| {
        Link::Uses(UsedLink {
            href: Href(x.into()),
            path: doc.path.clone(),
            paragraph: None,
        })
    };

    assert_eq!(
        &links.collect::<Vec<_>>(),
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
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(&arena, true, "../../ruby#foo"),
        Href("platforms/ruby#foo".into())
    );
    assert_eq!(
        doc.join(&arena, true, "../../ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );

    assert_eq!(
        doc.join(&arena, false, "/platforms/ruby"),
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(&arena, true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
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
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(&arena, true, "../ruby#foo"),
        Href("platforms/ruby#foo".into())
    );
    assert_eq!(
        doc.join(&arena, true, "../ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );

    assert_eq!(
        doc.join(&arena, false, "/platforms/ruby"),
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(&arena, true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );
}

#[test]
fn test_is_bad_schema() {
    assert!(is_bad_schema(b"//"));
    assert!(!is_bad_schema(b""));
    assert!(!is_bad_schema(b"http"));
    assert!(is_bad_schema(b"http:"));
    assert!(is_bad_schema(b"http:/"));
    assert!(!is_bad_schema(b"http/"));
}
