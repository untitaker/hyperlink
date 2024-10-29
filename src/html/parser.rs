use std::convert::Infallible;

use bumpalo::collections::String as BumpString;
use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use html5gum::callbacks::Callback;
use html5gum::callbacks::CallbackEvent;

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
}

impl ParserBuffers {
    pub fn reset(&mut self) {
        self.current_tag_name.clear();
        self.current_attribute_name.clear();
    }
}

pub struct HyperlinkVisitor<'a, 'l, 'd, P: ParagraphWalker> {
    pub paragraph_walker: P,
    pub arena: &'a Bump,
    pub document: &'d Document,
    pub link_buf: &'d mut BumpVec<'a, Link<'l, P::Paragraph>>,
    pub in_paragraph: bool,
    pub last_paragraph_i: usize,
    pub buffers: &'d mut ParserBuffers,
    pub check_anchors: bool,
}

impl<'a, 'l, 'd, P> Callback<Infallible> for HyperlinkVisitor<'a, 'l, 'd, P> where 'a: 'l, P: ParagraphWalker {
    fn handle_event(&mut self, event: CallbackEvent<'_>) -> Option<Infallible> {
        match event {
            CallbackEvent::OpenStartTag { name } => {
                self.buffers.current_tag_name.clear();
                self.buffers.current_tag_name.extend(name);
            }
            CallbackEvent::AttributeName { name } => {
                self.buffers.current_attribute_name.clear();
                self.buffers.current_attribute_name.extend(name);
            }
            CallbackEvent::AttributeValue { value } => {
                match (self.buffers.current_tag_name.as_slice(), self.buffers.current_attribute_name.as_slice()) {
                    (b"link" | b"area" | b"a", b"href") => self.extract_used_link(value),
                    (b"a", b"name") => self.extract_anchor_def(value),
                    (b"img" | b"script" | b"iframe", b"src") => self.extract_used_link(value),
                    (b"img", b"srcset") => self.extract_used_link_srcset(value),
                    (b"object", b"data") => self.extract_used_link(value),
                    (_, b"id") => self.extract_anchor_def(value),
                    _ => (),
                }

                self.buffers.current_attribute_name.clear();
            }
            CallbackEvent::CloseStartTag { .. } => {
                let is_paragraph_tag = !P::is_noop() && is_paragraph_tag(&self.buffers.current_tag_name);
                if is_paragraph_tag {
                    self.in_paragraph = true;
                    self.last_paragraph_i = self.link_buf.len();
                    self.paragraph_walker.finish_paragraph();
                }
                self.buffers.current_tag_name.clear();
                self.buffers.current_attribute_name.clear();
            }
            CallbackEvent::EndTag { name } => {
                let is_paragraph_tag = !P::is_noop() && is_paragraph_tag(name);
                if is_paragraph_tag {
                    let paragraph = self.paragraph_walker.finish_paragraph();
                    if self.in_paragraph {
                        for link in &mut self.link_buf[self.last_paragraph_i..] {
                            if let Link::Uses(ref mut x) = link {
                                x.paragraph = paragraph.clone();
                            }
                        }
                        self.in_paragraph = false;
                    }
                    self.last_paragraph_i = self.link_buf.len();
                }
                self.buffers.current_tag_name.clear();
            }
            CallbackEvent::String { .. } => {}
            // TODO: port should_emit_errors
            CallbackEvent::Error(_) => {}
            CallbackEvent::Comment { .. } => {}
            CallbackEvent::Doctype { .. } => {}
        }

        None
    }
}

impl<'a, 'l, 'd, P> HyperlinkVisitor<'a, 'l, 'd, P>
where
    'a: 'l,
    P: ParagraphWalker,
{
    fn extract_used_link(&mut self, attribute_value: &[u8]) {
        let value = try_normalize_href_value(
            std::str::from_utf8(&attribute_value).unwrap(),
        );

        self.link_buf.push(Link::Uses(UsedLink {
            href: self.document.join(self.arena, self.check_anchors, value),
            path: self.document.path.clone(),
            paragraph: None,
        }));
    }

    fn extract_used_link_srcset(&mut self, attribute_value: &[u8]) {
        let value = try_normalize_href_value(
            std::str::from_utf8(attribute_value).unwrap(),
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

    fn extract_anchor_def(&mut self, attribute_value: &[u8]) {
        if self.check_anchors {
            let mut href = BumpString::new_in(self.arena);
            let value = try_normalize_href_value(
                std::str::from_utf8(&attribute_value).unwrap(),
            );
            href.push('#');
            href.push_str(value);

            self.link_buf.push(Link::Defines(DefinedLink {
                href: self.document.join(self.arena, self.check_anchors, &href),
            }));
        }
    }

}
