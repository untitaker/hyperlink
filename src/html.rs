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
        if self
            .path
            .extension()
            .map_or(false, |extension| extension == "html" || extension == "htm")
        {
            self.links_from_read(File::open(&*self.path)?)
        } else {
            Ok(Vec::new())
        }
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
        buffer_queue.push_back(str_tendril); // XXX: actually drops data?
        debug_assert!(!buffer_queue.is_empty());

        let mut tokenizer = Tokenizer::new(LinkSink, Default::default());
        let mut links = Vec::new();

        'append: while let TokenizerResult::Script((href, lineno)) =
            tokenizer.feed(&mut buffer_queue)
        {
            for schema in &["http://", "https://", "irc://", "ftp://", "mailto:"] {
                if href.starts_with(schema) {
                    continue 'append;
                }
            }

            links.push(Link {
                href: self.join(href),
                lineno,
                path: self.path.clone(),
            });
        }

        Ok(links)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, derive_more::Display)]
#[display(fmt = "{}", "_0.display()")]
pub struct Href(PathBuf);

pub struct Link {
    pub href: Href,
    pub path: PathBuf,
    pub lineno: u64,
}

struct LinkSink;

impl TokenSink for LinkSink {
    type Handle = (StrTendril, u64);

    fn process_token(&mut self, token: Token, line_number: u64) -> TokenSinkResult<Self::Handle> {
        match token {
            Token::TagToken(tag) => {
                if &tag.name == "a" {
                    for attr in tag.attrs {
                        if &attr.name.local == "href" {
                            return TokenSinkResult::Script((attr.value, line_number));
                        }
                    }
                }
            }
            _ => (),
        }

        TokenSinkResult::Continue
    }
}
