use std::fs;
use std::io::BufReader;
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
    let mut base = "2019/".to_owned();
    let path = "../feed.xml";
    push_and_canonicalize(&mut base, path);
    assert_eq!(base, "feed.xml");
}

#[test]
fn test_push_and_canonicalize2() {
    let mut base = "contact.html".to_owned();
    let path = "contact.html";
    push_and_canonicalize(&mut base, path);
    assert_eq!(base, "contact.html");
}

#[test]
fn test_push_and_canonicalize3() {
    let mut base = "".to_owned();
    let path = "./2014/article.html";
    push_and_canonicalize(&mut base, path);
    assert_eq!(base, "2014/article.html");
}

#[derive(Debug, Clone, derive_more::Display, Eq, PartialEq, Ord, PartialOrd)]
pub struct Href(String);

impl Href {
    pub fn without_anchor(&self) -> Href {
        let mut s = self.0.clone();

        if let Some(i) = s.find('#') {
            s.truncate(i);
        }

        Href(s)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct UsedLink {
    pub href: Href,
    pub path: PathBuf,
    pub paragraph: Option<Paragraph>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct DefinedLink {
    pub href: Href,
    pub paragraph: Option<Paragraph>,
}

pub enum Link {
    Uses(UsedLink),
    Defines(DefinedLink),
}

pub struct Document {
    pub path: PathBuf,
    pub href: Href,
    pub is_index_html: bool,
}

impl Document {
    pub fn new(base_path: &Path, path: &Path) -> Self {
        let mut href_path = path
            .strip_prefix(base_path)
            .expect("base_path is not a base of path")
            .to_owned();

        let is_index_html = href_path.ends_with("index.html") || href_path.ends_with("index.htm");

        if is_index_html {
            href_path.pop();
        }

        let href = Href(href_path.display().to_string());
        let path = path.to_owned();

        Document {
            path,
            href,
            is_index_html,
        }
    }

    fn join(&self, preserve_anchor: bool, rel_href: &str) -> Href {
        let qs_start = rel_href
            .find(&['?', '#'][..])
            .unwrap_or_else(|| rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or_else(|| rel_href.len());

        let mut href = self.href.0.clone();
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

        Href(href)
    }

    pub fn links<F: FnMut(Link)>(
        &self,
        check_anchors: bool,
        get_paragraphs: bool,
        mut sink: F,
    ) -> Result<(), Error> {
        let mut reader = Reader::from_reader(BufReader::new(fs::File::open(&self.path)?));
        reader.trim_text(true);
        reader.expand_empty_elements(true);
        reader.check_end_names(false);

        // XXX: Move into threadlocal?
        let mut buf = Vec::new();

        let mut hasher = ParagraphHasher::new();
        let mut pending_links = Vec::new();
        let mut in_paragraph = false;

        loop {
            match reader.read_event(&mut buf)? {
                Event::Eof => break,
                Event::Start(ref e) => {
                    if PARAGRAPH_TAGS.contains(&e.name()) {
                        in_paragraph = true;

                        for link in pending_links.drain(..) {
                            sink(link);
                        }
                    }

                    macro_rules! extract_used_link {
                        ($attr_name:expr) => {
                            for attr in e.html_attributes() {
                                let attr = attr?;

                                if attr.key == $attr_name.as_bytes()
                                    && BAD_SCHEMAS
                                        .iter()
                                        .all(|schema| !attr.value.starts_with(schema.as_bytes()))
                                {
                                    pending_links.push(Link::Uses(UsedLink {
                                        href: self.join(
                                            check_anchors,
                                            str::from_utf8(&attr.unescaped_value()?)?,
                                        ),
                                        path: self.path.clone(),
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

                                    if attr.key == $attr_name.as_bytes() {
                                        pending_links.push(Link::Defines(DefinedLink {
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
                            extract_used_link!("href");
                            extract_anchor_def!("name");
                        }
                        b"img" => extract_used_link!("src"),
                        b"link" => extract_used_link!("href"),
                        b"script" => extract_used_link!("src"),
                        b"iframe" => extract_used_link!("src"),
                        b"area" => extract_used_link!("href"),
                        b"object" => extract_used_link!("data"),
                        _ => {}
                    }

                    extract_anchor_def!("id");
                }
                Event::End(e) if get_paragraphs => {
                    if PARAGRAPH_TAGS.contains(&e.name()) {
                        let paragraph = hasher.finish_paragraph();
                        for mut link in pending_links.drain(..) {
                            match link {
                                Link::Defines(ref mut x) => {
                                    x.paragraph = Some(paragraph);
                                }
                                Link::Uses(ref mut x) => {
                                    x.paragraph = Some(paragraph);
                                }
                            }
                            sink(link);
                        }
                        in_paragraph = false;
                    }
                }
                Event::Text(e) if get_paragraphs && in_paragraph => {
                    hasher.update(str::from_utf8(&e.unescaped()?)?);
                }
                _ => {}
            }
        }

        for link in pending_links {
            sink(link);
        }

        Ok(())
    }
}
