use std::fs;
use std::path::PathBuf;

use anyhow::Error;
use pulldown_cmark::{Event, Parser, Tag};

use crate::paragraph::{Paragraph, ParagraphHasher};

// Note: Keep in sync with html.rs
static PARAGRAPH_TAGS: &[Tag<'_>] = &[Tag::Paragraph, Tag::Item];

#[derive(Clone)]
pub struct DocumentSource {
    pub path: PathBuf,
}

impl DocumentSource {
    pub fn new(path: PathBuf) -> Self {
        DocumentSource { path }
    }

    pub fn paragraphs(&self, mut sink: impl FnMut(Paragraph)) -> Result<(), Error> {
        let text = fs::read_to_string(&self.path)?;

        let mut in_paragraph = false;
        let mut hasher = ParagraphHasher::new();

        for event in Parser::new(&text) {
            match event {
                Event::Start(tag) if PARAGRAPH_TAGS.contains(&tag) => {
                    in_paragraph = true;
                }
                Event::End(tag) if PARAGRAPH_TAGS.contains(&tag) => {
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
