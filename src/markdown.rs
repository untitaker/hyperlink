use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Error;
use pulldown_cmark::{Event, Parser, Tag};

use crate::paragraph::ParagraphWalker;

// Note: Keep in sync with html.rs
static PARAGRAPH_TAGS: &[Tag<'_>] = &[Tag::Paragraph, Tag::Item];

#[derive(Clone)]
pub struct DocumentSource {
    pub path: Arc<PathBuf>,
}

impl DocumentSource {
    pub fn new(path: PathBuf) -> Self {
        DocumentSource {
            path: Arc::new(path),
        }
    }

    pub fn paragraphs<P: ParagraphWalker>(&self) -> Result<Vec<P::Paragraph>, Error> {
        let text_raw = fs::read_to_string(&*self.path)?;
        let mut text = String::new();
        for mut line in text_raw.lines() {
            if line.starts_with('<') {
                continue;
            }

            if line.starts_with(": ") {
                line = &line[2..];
            }

            text.push_str(line);
            text.push('\n');
        }

        let mut in_paragraph = false;
        let mut walker = P::new();
        let mut rv = Vec::new();

        for event in Parser::new(&text) {
            match event {
                Event::Start(tag) if PARAGRAPH_TAGS.contains(&tag) => {
                    walker.finish_paragraph();
                    in_paragraph = true;
                }
                Event::End(tag) if PARAGRAPH_TAGS.contains(&tag) => {
                    let paragraph = walker.finish_paragraph();
                    if in_paragraph {
                        rv.extend(paragraph);
                    }
                    in_paragraph = false;
                }
                Event::Text(text) | Event::Code(text) => {
                    if in_paragraph {
                        walker.update(text.as_bytes());
                    }
                }
                _ => {}
            }
        }

        Ok(rv)
    }
}
