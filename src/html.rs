use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use html5ever::tendril::{ByteTendril, ReadExt, StrTendril};
use html5ever::tokenizer::{
    BufferQueue, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerResult,
};

fn force_relative(path: &Path) -> &Path {
    path.strip_prefix("/").unwrap_or(path)
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

    pub fn links(&self) -> io::Result<Vec<Link>> {
        self.links_from_read(File::open(&self.path)?)
    }

    fn join(&self, rel_href: StrTendril) -> Href {
        let rel_href = &rel_href[..];
        let trim_to = rel_href.find(&['#', '?'][..]).unwrap_or(rel_href.len());
        let rel_href = &rel_href[..trim_to];

        let unsanitized_href = force_relative(&self.href.0.join(rel_href)).to_owned();

        let unsanitized_href = self.base_path.join(unsanitized_href);

        let sanitized_href = unsanitized_href
            .canonicalize()
            // XXX: if the link does not exist, this will fail. In that case the link error will be
            // reported with `..` in it, but that's fine.
            .unwrap_or_else(|_| unsanitized_href)
            .strip_prefix(&self.base_path)
            .expect("base_path is not a base of path")
            .to_owned();

        Href(sanitized_href)
    }

    fn links_from_read<R: io::Read>(&self, mut readable: R) -> io::Result<Vec<Link>> {
        let mut byte_tendril = ByteTendril::new();
        readable.read_to_tendril(&mut byte_tendril)?;

        let str_tendril = byte_tendril.try_reinterpret().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file did not contain valid UTF-8",
            )
        })?;

        let mut buffer_queue = BufferQueue::new();
        buffer_queue.push_back(str_tendril);

        let mut links = Vec::new();
        let mut tokenizer = Tokenizer::new(
            LinkSink {
                into: &mut links,
                document: &self,
            },
            Default::default(),
        );

        loop {
            if matches!(tokenizer.feed(&mut buffer_queue), TokenizerResult::Done) {
                break;
            }
        }

        Ok(links)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, derive_more::Display, Ord, PartialOrd)]
#[display(fmt = "{}", "_0.display()")]
pub struct Href(PathBuf);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Link {
    pub href: Href,
    pub path: PathBuf,
    pub lineno: u64,
}

struct LinkSink<'a> {
    into: &'a mut Vec<Link>,
    document: &'a Document,
}

static BAD_SCHEMAS: &[&str] = &[
    "http://", "https://", "irc://", "ftp://", "mailto:", "data:",
];

impl<'a> TokenSink for LinkSink<'a> {
    type Handle = ();

    fn process_token(&mut self, token: Token, line_number: u64) -> TokenSinkResult<Self::Handle> {
        match token {
            Token::TagToken(tag) => {
                macro_rules! extract_tag {
                    ($tag_name:expr, $attr_name:expr) => {
                        if &tag.name == $tag_name {
                            let document = &self.document;
                            self.into.extend(
                                tag.attrs
                                    .into_iter()
                                    .filter(|attr| {
                                        &attr.name.local == $attr_name
                                            && BAD_SCHEMAS
                                                .iter()
                                                .all(|schema| !attr.value.starts_with(schema))
                                    })
                                    .map(|attr| Link {
                                        href: document.join(attr.value),
                                        lineno: line_number,
                                        path: document.path.clone(),
                                    }),
                            );
                            return TokenSinkResult::Continue;
                        }
                    };
                }

                extract_tag!("a", "href");
                extract_tag!("img", "src");
                extract_tag!("link", "href");
                extract_tag!("script", "src");
                extract_tag!("script", "src");
            }
            _ => (),
        }

        TokenSinkResult::Continue
    }
}
