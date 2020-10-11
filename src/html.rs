use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use anyhow::Error;
use xmlparser::{ElementEnd, Token, Tokenizer};

use crate::paragraph::{Paragraph, ParagraphHasher};

static BAD_SCHEMAS: &[&str] = &[
    "http://", "https://", "irc://", "ftp://", "mailto:", "data:",
];

#[inline]
fn push_and_canonicalize(base: &mut String, path: &str) {
    if path.starts_with('/') {
        base.clear();
    }

    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if let Some(i) = base.rfind('/') {
                    base.truncate(i);
                }
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

#[derive(Debug, Clone, Eq, PartialEq, Hash, derive_more::Display, Ord, PartialOrd)]
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

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct UsedLink {
    pub href: Href,
    pub path: PathBuf,
    pub paragraph: Option<Paragraph>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
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
}

impl Document {
    pub fn new(base_path: &Path, path: &Path) -> Self {
        let mut href_path = path
            .strip_prefix(base_path)
            .expect("base_path is not a base of path")
            .to_owned();

        if href_path.ends_with("index.html") || href_path.ends_with("index.htm") {
            href_path.pop();
        }

        let href = Href(href_path.display().to_string());
        let path = path.to_owned();

        Document { path, href }
    }

    fn join(&self, preserve_anchor: bool, rel_href: &str) -> Href {
        let qs_start = rel_href
            .find(&['?', '#'][..])
            .unwrap_or_else(|| rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or_else(|| rel_href.len());

        let mut href = self.href.0.clone();
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
        let text = fs::read_to_string(&self.path)?;

        let mut current_tag = CurrentTag::None;
        let mut in_paragraph = false;
        let mut hasher = ParagraphHasher::new();
        let mut pending_links = Vec::new();

        for token in Tokenizer::from(text.as_str()) {
            match &token? {
                Token::ElementStart { local, .. } => {
                    if &**local == "p" {
                        in_paragraph = true;

                        for link in pending_links.drain(..) {
                            sink(link);
                        }
                    }

                    current_tag = CurrentTag::from(&**local);
                }
                Token::ElementEnd { end, .. } if get_paragraphs => {
                    if let ElementEnd::Close(_, tag_name) = end {
                        if &**tag_name == "p" {
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

                    current_tag = CurrentTag::None;
                }
                Token::Attribute { local, value, .. } => {
                    macro_rules! extract_used_link {
                        ($attr_name:expr) => {
                            if &**local == $attr_name
                                && BAD_SCHEMAS.iter().all(|schema| !value.starts_with(schema))
                            {
                                pending_links.push(Link::Uses(UsedLink {
                                    href: self.join(check_anchors, value),
                                    path: self.path.clone(),
                                    paragraph: None,
                                }));
                            }
                        };
                    }

                    macro_rules! extract_anchor_def {
                        ($attr_name:expr) => {
                            if check_anchors && &**local == $attr_name {
                                pending_links.push(Link::Defines(DefinedLink {
                                    href: self.join(check_anchors, &format!("#{}", value)),
                                    paragraph: None,
                                }));
                            }
                        };
                    }

                    match current_tag {
                        CurrentTag::A => {
                            extract_used_link!("href");
                            extract_anchor_def!("name");
                        }
                        CurrentTag::Img => extract_used_link!("src"),
                        CurrentTag::Link => extract_used_link!("href"),
                        CurrentTag::Script => extract_used_link!("src"),
                        CurrentTag::IFrame => extract_used_link!("src"),
                        CurrentTag::Area => extract_used_link!("href"),
                        CurrentTag::Object => extract_used_link!("data"),
                        CurrentTag::None => {}
                    }

                    extract_anchor_def!("id");
                }
                Token::Text { text } if get_paragraphs && in_paragraph => {
                    hasher.update(text);
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

#[derive(Clone, Copy)]
enum CurrentTag {
    None,
    A,
    Img,
    Link,
    Script,
    IFrame,
    Area,
    Object,
}

impl From<&str> for CurrentTag {
    fn from(s: &str) -> CurrentTag {
        match s {
            "a" => CurrentTag::A,
            "img" => CurrentTag::Img,
            "link" => CurrentTag::Link,
            "script" => CurrentTag::Script,
            "iframe" => CurrentTag::IFrame,
            "area" => CurrentTag::Area,
            "object" => CurrentTag::Object,
            _ => CurrentTag::None,
        }
    }
}
