use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Error;
use pulldown_cmark::{Event, Parser, TagEnd};

use crate::paragraph::ParagraphWalker;

// Note: Keep in sync with html.rs
static PARAGRAPH_TAGS: &[TagEnd] = &[TagEnd::Paragraph, TagEnd::Item];

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

    pub fn paragraphs<P: ParagraphWalker>(&self) -> Result<Vec<(P::Paragraph, usize)>, Error> {
        let mut text = String::new();
        // line_numbers[0] = 32 ... line 0 ends at `text` offset 32
        let mut line_numbers = Vec::new();
        for line in BufReader::new(File::open(&*self.path)?).lines() {
            let line = line?;
            let mut line = line.as_str();

            if line.starts_with('<') {
                continue;
            }

            if line.starts_with(": ") {
                line = &line[2..];
            }

            text.push_str(line);
            text.push('\n');
            line_numbers.push(text.len());
        }

        let mut in_paragraph = false;
        let mut walker = P::new();
        let mut rv = Vec::new();

        for (event, range) in Parser::new(&text).into_offset_iter() {
            match event {
                Event::Start(tag) if PARAGRAPH_TAGS.contains(&tag.to_end()) => {
                    walker.finish_paragraph();
                    in_paragraph = true;
                }
                Event::End(tag) if PARAGRAPH_TAGS.contains(&tag) => {
                    let paragraph = walker.finish_paragraph();
                    if in_paragraph {
                        if let Some(paragraph) = paragraph {
                            let lineno = match line_numbers.binary_search(&range.end) {
                                Ok(i) => i + 1,
                                Err(i) => i + 1,
                            };
                            rv.push((paragraph, lineno));
                        }
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
