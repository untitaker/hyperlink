use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str;

use anyhow::Error;
use xmlparser::{Token, Tokenizer};

static BAD_SCHEMAS: &[&str] = &[
    "http://", "https://", "irc://", "ftp://", "mailto:", "data:",
];

fn force_relative(path: &Path) -> &Path {
    path.strip_prefix("/").unwrap_or(path)
}

/// A version of fs::canonicalize that just resolves ../ and therefore does no IO
fn simple_canonicalize(path: &Path) -> PathBuf {
    let mut rv = PathBuf::new();

    for component in path.components() {
        if component == Component::ParentDir {
            rv.pop();
        } else {
            rv.push(component);
        }
    }

    rv
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, derive_more::Display, Ord, PartialOrd)]
#[display(fmt = "{}", "_0.display()")]
pub struct Href(PathBuf);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Link {
    pub href: Href,
    pub path: PathBuf,
}

pub struct Document {
    pub base_path: PathBuf,
    pub path: PathBuf,
    pub href: Href,
}

impl Document {
    pub fn new(base_path: &Path, path: &Path) -> Self {
        let href = {
            let mut href_path = path
                .strip_prefix(base_path)
                .expect("base_path is not a base of path")
                .to_owned();

            if href_path.ends_with("index.html") || href_path.ends_with("index.htm") {
                href_path.pop();
            }

            Href(href_path)
        };

        let base_path = base_path.to_owned();
        let path = path.to_owned();

        Document {
            base_path,
            path,
            href,
        }
    }

    fn join(&self, rel_href: &str) -> Href {
        let trim_to = rel_href.find(&['#', '?'][..]).unwrap_or(rel_href.len());
        let rel_href = &rel_href[..trim_to];

        let unsanitized_href = self.href.0.join(rel_href);
        let unsanitized_href = force_relative(&unsanitized_href);
        let sanitized_href = simple_canonicalize(unsanitized_href);

        Href(sanitized_href.to_owned())
    }

    pub fn links<F: FnMut(Link)>(&self, mut sink: F) -> Result<(), Error> {
        let text = fs::read_to_string(&self.path)?;

        let mut current_tag = None;

        for token in Tokenizer::from(text.as_str()) {
            let token = token?;

            if let Token::ElementEnd { .. } = &token {
                current_tag = None;
                continue;
            }

            macro_rules! extract_tag {
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
                            sink(Link {
                                href: self.join(value),
                                path: self.path.clone(),
                            });
                        }
                    }
                };
            }

            extract_tag!("a", "href");
            extract_tag!("img", "src");
            extract_tag!("link", "href");
            extract_tag!("script", "src");
            extract_tag!("iframe", "src");
            extract_tag!("area", "href");
            extract_tag!("object", "data");
        }

        Ok(())
    }
}
