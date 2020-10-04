use std::fs;
use std::path::{Path, PathBuf};
use std::str;

use anyhow::Error;
use xmlparser::{Token, Tokenizer};

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
}

pub enum Link {
    Uses(UsedLink),
    Defines(Href),
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
        let qs_start = rel_href.find(&['?', '#'][..]).unwrap_or(rel_href.len());
        let anchor_start = rel_href.find('#').unwrap_or(rel_href.len());

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

    pub fn links<F: FnMut(Link)>(&self, check_anchors: bool, mut sink: F) -> Result<(), Error> {
        let text = fs::read_to_string(&self.path)?;

        let mut current_tag = None;

        for token in Tokenizer::from(text.as_str()) {
            let token = token?;

            if let Token::ElementEnd { .. } = &token {
                current_tag = None;
                continue;
            }

            macro_rules! extract_used_link {
                ($tag_name:expr, $attr_name:expr) => {
                    if let Token::ElementStart { local, .. } = &token {
                        if &**local == $tag_name {
                            current_tag = Some($tag_name);
                            continue;
                        }
                    }

                    if let Token::Attribute { local, value, .. } = &token {
                        if current_tag == Some($tag_name)
                            && &**local == $attr_name
                            && BAD_SCHEMAS.iter().all(|schema| !value.starts_with(schema))
                        {
                            sink(Link::Uses(UsedLink {
                                href: self.join(check_anchors, value),
                                path: self.path.clone(),
                            }));
                            continue;
                        }
                    }
                };
            }

            macro_rules! extract_anchor_def {
                ($tag_name:expr, $attr_name:expr) => {
                    if check_anchors {
                        if let Token::ElementStart { local, .. } = &token {
                            if &**local == $tag_name {
                                current_tag = Some($tag_name);
                                continue;
                            }
                        }

                        if let Token::Attribute { local, value, .. } = &token {
                            if current_tag == Some($tag_name) && &**local == $attr_name {
                                sink(Link::Defines(
                                    self.join(check_anchors, &format!("#{}", value)),
                                ));
                                continue;
                            }
                        }
                    }
                };
            }

            extract_used_link!("a", "href");
            extract_used_link!("img", "src");
            extract_used_link!("link", "href");
            extract_used_link!("script", "src");
            extract_used_link!("iframe", "src");
            extract_used_link!("area", "href");
            extract_used_link!("object", "data");

            extract_anchor_def!("a", "name");
            extract_anchor_def!("h1", "id");
            extract_anchor_def!("h2", "id");
            extract_anchor_def!("h3", "id");
            extract_anchor_def!("h4", "id");
            extract_anchor_def!("h5", "id");
            extract_anchor_def!("h6", "id");
        }

        Ok(())
    }
}
