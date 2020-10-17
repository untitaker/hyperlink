use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str;

use anyhow::Error;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::paragraph::{Paragraph, ParagraphHasher};

static BAD_SCHEMAS: &[&str] = &[
    "http://", "https://", "irc://", "ftp://", "mailto:", "data:",
];

static PARAGRAPH_TAGS: &[&[u8]] = &[b"p", b"li"];

#[inline]
fn push_and_canonicalize(base: &mut String, path: &str) {
    if path.starts_with('/') {
        base.clear();
    }

    base.truncate(base.rfind('/').unwrap_or(0));

    for component in path.split('/') {
        match component {
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

#[test]
fn test_push_and_canonicalize() {
    let mut base = "2019/".into();
    let path = "../feed.xml";
    push_and_canonicalize(&mut base, path);
    assert_eq!(base, "feed.xml");
}

#[test]
fn test_push_and_canonicalize2() {
    let mut base = "contact.html".into();
    let path = "contact.html";
    push_and_canonicalize(&mut base, path);
    assert_eq!(base, "contact.html");
}

#[test]
fn test_push_and_canonicalize3() {
    let mut base = "".into();
    let path = "./2014/article.html";
    push_and_canonicalize(&mut base, path);
    assert_eq!(base, "2014/article.html");
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Href<'a>(Cow<'a, str>);

impl<'a> Href<'a> {
    pub fn without_anchor(&self) -> Href<'_> {
        let mut s = &self.0[..];

        if let Some(i) = s.find('#') {
            s = &s[..i];
        }

        Href(s.into())
    }
}

impl<'a> fmt::Display for Href<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct UsedLink<'a> {
    pub href: Href<'a>,
    pub path: &'a Path,
    pub paragraph: Option<Paragraph>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct DefinedLink<'a> {
    pub href: Href<'a>,
    pub paragraph: Option<Paragraph>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum Link<'a> {
    Uses(UsedLink<'a>),
    Defines(DefinedLink<'a>),
}

pub struct Document {
    pub path: PathBuf,
    pub href: Href<'static>,
    pub is_index_html: bool,
}

impl Document {
    pub fn new(base_path: &Path, path: PathBuf) -> Self {
        let mut href_path = path
            .strip_prefix(base_path)
            .expect("base_path is not a base of path");

        let is_index_html = href_path.ends_with("index.html") || href_path.ends_with("index.htm");

        if is_index_html {
            href_path = href_path.parent().unwrap_or(href_path);
        }

        let mut href = href_path.display().to_string();
        if cfg!(windows) {
            href = href.replace('\\', "/");
        }

        let href = Href(href.into());

        Document {
            path,
            href,
            is_index_html,
        }
    }

    fn join(&self, preserve_anchor: bool, rel_href: &str) -> Href<'_> {
        let qs_start = rel_href
            .find(&['?', '#'][..])
            .unwrap_or_else(|| rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or_else(|| rel_href.len());

        let mut href = self.href.0.clone().into_owned();
        if self.is_index_html {
            href.push('/');
        }

        push_and_canonicalize(&mut href, &rel_href[..qs_start]);

        if preserve_anchor {
            let anchor = &rel_href[anchor_start..];
            if anchor.len() > 1 {
                href.push_str(anchor);
            }
        }

        Href(href.into())
    }

    pub fn links<'a>(
        &'a self,
        buf: &mut Vec<u8>,
        check_anchors: bool,
        get_paragraphs: bool,
    ) -> Result<Vec<Link<'a>>, Error> {
        self.links_from_read(
            buf,
            fs::File::open(&self.path)?,
            check_anchors,
            get_paragraphs,
        )
    }

    fn links_from_read<'a>(
        &'a self,
        buf: &mut Vec<u8>,
        read: impl Read,
        check_anchors: bool,
        get_paragraphs: bool,
    ) -> Result<Vec<Link<'a>>, Error> {
        let mut reader = Reader::from_reader(BufReader::new(read));
        reader.trim_text(true);
        reader.expand_empty_elements(true);
        reader.check_end_names(false);

        let mut hasher = ParagraphHasher::new();
        let mut rv = Vec::new();
        let mut last_paragraph_i = 0;
        let mut in_paragraph = false;

        loop {
            match reader.read_event(buf)? {
                Event::Eof => break,
                Event::Start(ref e) => {
                    if PARAGRAPH_TAGS.contains(&e.name()) {
                        in_paragraph = true;
                        last_paragraph_i = rv.len();
                    }

                    macro_rules! extract_used_link {
                        ($attr_name:expr) => {
                            for attr in e.html_attributes() {
                                let attr = attr?;

                                if attr.key == $attr_name
                                    && BAD_SCHEMAS
                                        .iter()
                                        .all(|schema| !attr.value.starts_with(schema.as_bytes()))
                                {
                                    rv.push(Link::Uses(UsedLink {
                                        href: self.join(
                                            check_anchors,
                                            str::from_utf8(&attr.unescaped_value()?)?,
                                        ),
                                        path: &self.path,
                                        paragraph: None,
                                    }));
                                }
                            }
                        };
                    }

                    macro_rules! extract_anchor_def {
                        ($attr_name:expr) => {
                            if check_anchors {
                                for attr in e.html_attributes() {
                                    let attr = attr?;

                                    if attr.key == $attr_name {
                                        rv.push(Link::Defines(DefinedLink {
                                            href: self.join(
                                                check_anchors,
                                                &format!("#{}", str::from_utf8(&attr.value)?),
                                            ),
                                            paragraph: None,
                                        }));
                                    }
                                }
                            }
                        };
                    }

                    match e.name() {
                        b"a" => {
                            extract_used_link!(b"href");
                            extract_anchor_def!(b"name");
                        }
                        b"img" => extract_used_link!(b"src"),
                        b"link" => extract_used_link!(b"href"),
                        b"script" => extract_used_link!(b"src"),
                        b"iframe" => extract_used_link!(b"src"),
                        b"area" => extract_used_link!(b"href"),
                        b"object" => extract_used_link!(b"data"),
                        _ => {}
                    }

                    extract_anchor_def!(b"id");
                }
                Event::End(e) if get_paragraphs => {
                    if PARAGRAPH_TAGS.contains(&e.name()) {
                        let paragraph = hasher.finish_paragraph();
                        for link in &mut rv[last_paragraph_i..] {
                            match link {
                                Link::Defines(ref mut x) => {
                                    x.paragraph = Some(paragraph);
                                }
                                Link::Uses(ref mut x) => {
                                    x.paragraph = Some(paragraph);
                                }
                            }
                        }
                        in_paragraph = false;
                        last_paragraph_i = rv.len();
                    }
                }
                Event::Text(e) if get_paragraphs && in_paragraph => {
                    hasher.update(str::from_utf8(&e.unescaped()?)?);
                }
                _ => {}
            }
        }

        buf.clear();

        Ok(rv)
    }
}

#[test]
fn test_document_href() {
    let doc = Document::new(
        Path::new("public/"),
        "public/platforms/python/troubleshooting/index.html".into(),
    );

    assert_eq!(doc.href, Href("platforms/python/troubleshooting".into()));

    let doc = Document::new(
        Path::new("public/"),
        "public/platforms/python/troubleshooting.html".into(),
    );

    assert_eq!(
        doc.href,
        Href("platforms/python/troubleshooting.html".into())
    );
}

#[test]
fn test_document_links() {
    let doc = Document::new(
        Path::new("public/"),
        "public/platforms/python/troubleshooting/index.html".into(),
    );

    let links = doc
        .links_from_read(
            &mut Vec::new(),
            r#"""
    <a href="../../ruby/" />
    <a href="/platforms/perl/">Perl</a>

    <a href=../../rust/>
    <a href='../../go/'>
    """#
            .as_bytes(),
            false,
            false,
        )
        .unwrap();

    let used_link = |x: &'static str| {
        Link::Uses(UsedLink {
            href: Href(x.into()),
            path: &doc.path,
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
        ]
    );
}

#[test]
fn test_document_join_index_html() {
    let doc = Document::new(
        Path::new("public/"),
        "public/platforms/python/troubleshooting/index.html".into(),
    );

    assert_eq!(
        doc.join(false, "../../ruby#foo"),
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(true, "../../ruby#foo"),
        Href("platforms/ruby#foo".into())
    );
    assert_eq!(
        doc.join(true, "../../ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );

    assert_eq!(
        doc.join(false, "/platforms/ruby"),
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );
}

#[test]
fn test_document_join_bare_html() {
    let doc = Document::new(
        Path::new("public/"),
        "public/platforms/python/troubleshooting.html".into(),
    );

    assert_eq!(
        doc.join(false, "../ruby#foo"),
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(true, "../ruby#foo"),
        Href("platforms/ruby#foo".into())
    );
    assert_eq!(
        doc.join(true, "../ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );

    assert_eq!(
        doc.join(false, "/platforms/ruby"),
        Href("platforms/ruby".into())
    );
    assert_eq!(
        doc.join(true, "/platforms/ruby?bar=1#foo"),
        Href("platforms/ruby#foo".into())
    );
}
