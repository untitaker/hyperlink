use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Error;
use pulldown_cmark::{Event, Parser, Tag};

use crate::paragraph::{Paragraph, ParagraphHasher};

#[derive(Clone)]
pub struct DocumentSource {
    pub path: PathBuf,
}

impl DocumentSource {
    pub fn new(path: &Path) -> Self {
        DocumentSource {
            path: path.to_owned(),
        }
    }

    pub fn paragraphs<F: FnMut(Paragraph)>(&self, mut sink: F) -> Result<(), Error> {
        let text = fs::read_to_string(&self.path)?;

        let mut in_paragraph = false;
        let mut hasher = ParagraphHasher::new();

        for event in Parser::new(&text) {
            match event {
                Event::Start(Tag::Paragraph) => {
                    in_paragraph = true;
                }
                Event::End(Tag::Paragraph) => {
                    sink(hasher.finish_paragraph());
                    in_paragraph = false;
                }
                Event::Text(text) | Event::Code(text) => {
                    if in_paragraph {
                        hasher.update(text.as_ref());
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}
