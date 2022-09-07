use bumpalo::collections::String as BumpString;
use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use html5gum::{Emitter, Error, State};

use crate::html::{DefinedLink, Document, Link, UsedLink};
use crate::paragraph::ParagraphWalker;

#[inline]
fn is_paragraph_tag(tag: &[u8]) -> bool {
    tag == b"p" || tag == b"li" || tag == b"dt" || tag == b"dd"
}

#[inline]
fn try_normalize_href_value(input: &str) -> &str {
    input.trim()
}

#[derive(Default)]
pub struct ParserBuffers {
    current_tag_name: Vec<u8>,
    current_attribute_name: Vec<u8>,
    current_attribute_value: Vec<u8>,
    last_start_tag: Vec<u8>,
}

impl ParserBuffers {
    pub fn reset(&mut self) {
        self.current_tag_name.clear();
        self.current_attribute_name.clear();
        self.current_attribute_value.clear();
        self.last_start_tag.clear();
    }
}

pub struct HyperlinkEmitter<'a, 'l, 'd, P: ParagraphWalker> {
    pub paragraph_walker: P,
    pub arena: &'a Bump,
    pub document: &'d Document,
    pub link_buf: &'d mut BumpVec<'a, Link<'l, P::Paragraph>>,
    pub in_paragraph: bool,
    pub last_paragraph_i: usize,
    pub get_paragraphs: bool,
    pub buffers: &'d mut ParserBuffers,
    pub current_tag_is_closing: bool,
    pub check_anchors: bool,
}

impl<'a, 'l, 'd, P> HyperlinkEmitter<'a, 'l, 'd, P>
where
    'a: 'l,
    P: ParagraphWalker,
{
    fn extract_used_link(&mut self) {
        let value = try_normalize_href_value(
            std::str::from_utf8(&self.buffers.current_attribute_value).unwrap(),
        );

        self.link_buf.push(Link::Uses(UsedLink {
            href: self.document.join(self.arena, self.check_anchors, value),
            path: self.document.path.clone(),
            paragraph: None,
        }));
    }

    fn extract_used_link_srcset(&mut self) {
        let value = try_normalize_href_value(
            std::str::from_utf8(&self.buffers.current_attribute_value).unwrap(),
        );

        // https://html.spec.whatwg.org/multipage/images.html#srcset-attribute
        for value in value
            .split(',')
            .filter_map(|candidate: &str| candidate.split_whitespace().next())
            .filter(|value| !value.is_empty())
        {
            self.link_buf.push(Link::Uses(UsedLink {
                href: self.document.join(self.arena, self.check_anchors, value),
                path: self.document.path.clone(),
                paragraph: None,
            }));
        }
    }

    fn extract_anchor_def(&mut self) {
        if self.check_anchors {
            let mut href = BumpString::new_in(self.arena);
            let value = try_normalize_href_value(
                std::str::from_utf8(&self.buffers.current_attribute_value).unwrap(),
            );
            href.push('#');
            href.push_str(value);

            self.link_buf.push(Link::Defines(DefinedLink {
                href: self.document.join(self.arena, self.check_anchors, &href),
            }));
        }
    }

    fn flush_old_attribute(&mut self) {
        match (
            self.buffers.current_tag_name.as_slice(),
            self.buffers.current_attribute_name.as_slice(),
        ) {
            (b"link" | b"area" | b"a", b"href") => self.extract_used_link(),
            (b"a", b"name") => self.extract_anchor_def(),
            (b"img" | b"script" | b"iframe", b"src") => self.extract_used_link(),
            (b"img", b"srcset") => self.extract_used_link_srcset(),
            (b"object", b"data") => self.extract_used_link(),
            (_, b"id") => self.extract_anchor_def(),
            _ => (),
        }

        self.buffers.current_attribute_name.clear();
        self.buffers.current_attribute_value.clear();
    }
}

impl<'a, 'l, 'd, P> Emitter for HyperlinkEmitter<'a, 'l, 'd, P>
where
    'a: 'l,
    P: ParagraphWalker,
{
    type Token = ();

    fn set_last_start_tag(&mut self, last_start_tag: Option<&[u8]>) {
        self.buffers.last_start_tag.clear();
        self.buffers
            .last_start_tag
            .extend(last_start_tag.unwrap_or_default());
    }

    fn pop_token(&mut self) -> Option<()> {
        None
    }

    fn emit_string(&mut self, c: &[u8]) {
        if self.get_paragraphs && self.in_paragraph {
            self.paragraph_walker.update(c);
        }
    }

    fn init_start_tag(&mut self) {
        self.buffers.current_tag_name.clear();
        self.current_tag_is_closing = false;
    }

    fn init_end_tag(&mut self) {
        self.buffers.current_tag_name.clear();
        self.current_tag_is_closing = true;
    }

    fn emit_current_tag(&mut self) -> Option<State> {
        self.flush_old_attribute();

        self.buffers.last_start_tag.clear();
        if !self.current_tag_is_closing {
            self.buffers
                .last_start_tag
                .extend(&self.buffers.current_tag_name);

            if is_paragraph_tag(&self.buffers.current_tag_name) {
                self.in_paragraph = true;
                self.last_paragraph_i = self.link_buf.len();
                self.paragraph_walker.finish_paragraph();
            }
        } else if is_paragraph_tag(&self.buffers.current_tag_name) {
            let paragraph = self.paragraph_walker.finish_paragraph();
            if self.in_paragraph {
                for link in &mut self.link_buf[self.last_paragraph_i..] {
                    match link {
                        Link::Uses(ref mut x) => {
                            x.paragraph = paragraph.clone();
                        }
                        Link::Defines(_) => (),
                    }
                }
                self.in_paragraph = false;
            }
            self.last_paragraph_i = self.link_buf.len();
        }

        self.buffers.current_tag_name.clear();
        html5gum::naive_next_state(&self.buffers.last_start_tag)
    }

    fn set_self_closing(&mut self) {
        if is_paragraph_tag(&self.buffers.current_tag_name) {
            self.in_paragraph = false;
        }
    }

    fn push_tag_name(&mut self, s: &[u8]) {
        self.buffers.current_tag_name.extend(s);
    }

    fn init_attribute(&mut self) {
        self.flush_old_attribute();
    }

    fn push_attribute_name(&mut self, s: &[u8]) {
        self.buffers.current_attribute_name.extend(s);
    }

    fn push_attribute_value(&mut self, s: &[u8]) {
        self.buffers.current_attribute_value.extend(s);
    }

    fn current_is_appropriate_end_tag_token(&mut self) -> bool {
        self.current_tag_is_closing
            && !self.buffers.current_tag_name.is_empty()
            && self.buffers.current_tag_name == self.buffers.last_start_tag
    }

    fn emit_current_comment(&mut self) {}
    fn emit_current_doctype(&mut self) {}
    fn emit_eof(&mut self) {}
    fn emit_error(&mut self, _: Error) {}
    fn init_comment(&mut self) {}
    fn init_doctype(&mut self) {}
    fn push_comment(&mut self, _: &[u8]) {}
    fn push_doctype_name(&mut self, _: &[u8]) {}
    fn push_doctype_public_identifier(&mut self, _: &[u8]) {}
    fn push_doctype_system_identifier(&mut self, _: &[u8]) {}
    fn set_doctype_public_identifier(&mut self, _: &[u8]) {}
    fn set_doctype_system_identifier(&mut self, _: &[u8]) {}
    fn set_force_quirks(&mut self) {}
}

