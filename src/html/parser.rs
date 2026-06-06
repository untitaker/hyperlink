use bumpalo::Bump;
use html5gum::{Emitter, Error, State};

use crate::html::{DefinedLink, Document, Link, UsedLink};
use crate::paragraph::{ParagraphWalker, VoidParagraph};

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

/// Turns attribute values into links and emits them through the callback, keeping track of which
/// paragraph they belong to.
///
/// This lives in its own struct (disjoint from the tag/attribute buffers in HyperlinkEmitter) so
/// that links can be pushed while attribute values are still borrowed.
struct LinkExtractor<'a, 'd, P: ParagraphWalker, F> {
    paragraph_walker: P,
    arena: &'a Bump,
    document: &'d Document,
    /// Buffers the links of the currently open paragraph, see push_link. Lent out by
    /// DocumentBuffers so the allocation is reused across documents. Buffered links have no
    /// paragraph yet, it is attached in flush_links.
    link_buf: &'d mut Vec<Link<'a, VoidParagraph>>,
    scratch: &'d mut String,
    in_paragraph: bool,
    check_anchors: bool,
    callback: F,
}

impl<'a, P, F> LinkExtractor<'a, '_, P, F>
where
    P: ParagraphWalker,
    F: FnMut(Link<'a, P::Paragraph>),
{
    /// Links inside a paragraph are buffered until the paragraph is closed (their paragraph hash
    /// is assigned retroactively in flush_links), all other links go straight to the callback.
    ///
    /// If P is a noop walker, no paragraphs are tracked and link_buf is never used.
    fn push_link(&mut self, link: Link<'a, VoidParagraph>) {
        if !P::is_noop() && self.in_paragraph {
            self.link_buf.push(link);
        } else {
            (self.callback)(link.with_paragraph(None));
        }
    }

    fn flush_links(&mut self, paragraph: Option<P::Paragraph>) {
        if P::is_noop() {
            return;
        }

        for link in self.link_buf.drain(..) {
            (self.callback)(link.with_paragraph(paragraph.clone()));
        }
    }

    /// Begin a new paragraph. If a previous paragraph tag was left unclosed (e.g. the first link
    /// in <li><a href=..><p>..</p></li> -- html5gum is not a tree builder, so there are no
    /// implied end tags), it is ended here and its links get the paragraph text accumulated up
    /// to this point.
    fn begin_paragraph(&mut self) {
        self.end_paragraph();
        self.in_paragraph = true;
    }

    fn end_paragraph(&mut self) {
        if self.in_paragraph {
            let paragraph = self.paragraph_walker.finish_paragraph();
            self.in_paragraph = false;
            self.flush_links(paragraph);
        } else {
            // the walker only accumulates text while in_paragraph (see emit_string) and links
            // are only buffered inside paragraphs, so there is nothing to finish here
            debug_assert!(self.link_buf.is_empty());
        }
    }

    fn extract_used_link(&mut self, value: &[u8]) {
        let value = try_normalize_href_value(std::str::from_utf8(value).unwrap());

        let href = self
            .document
            .join(self.arena, self.scratch, self.check_anchors, value);
        self.push_link(Link::Uses(UsedLink {
            href,
            path: self.document.path.clone(),
            paragraph: None,
        }));
    }

    fn extract_used_link_srcset(&mut self, value: &[u8]) {
        let value = try_normalize_href_value(std::str::from_utf8(value).unwrap());

        // https://html.spec.whatwg.org/multipage/images.html#srcset-attribute
        for value in value
            .split(',')
            .filter_map(|candidate: &str| candidate.split_whitespace().next())
            .filter(|value| !value.is_empty())
        {
            let href = self
                .document
                .join(self.arena, self.scratch, self.check_anchors, value);
            self.push_link(Link::Uses(UsedLink {
                href,
                path: self.document.path.clone(),
                paragraph: None,
            }));
        }
    }

    fn extract_anchor_def(&mut self, value: &[u8]) {
        if self.check_anchors {
            let value = try_normalize_href_value(std::str::from_utf8(value).unwrap());

            let href = self.document.join_anchor(self.arena, self.scratch, value);
            self.push_link(Link::Defines(DefinedLink { href }));
        }
    }
}

pub struct HyperlinkEmitter<'a, 'd, P: ParagraphWalker, F> {
    extractor: LinkExtractor<'a, 'd, P, F>,
    buffers: &'d mut ParserBuffers,
    current_tag_is_closing: bool,
}

impl<'a, 'd, P, F> HyperlinkEmitter<'a, 'd, P, F>
where
    P: ParagraphWalker,
    F: FnMut(Link<'a, P::Paragraph>),
{
    pub fn new(
        arena: &'a Bump,
        document: &'d Document,
        buffers: &'d mut ParserBuffers,
        scratch: &'d mut String,
        link_buf: &'d mut Vec<Link<'a, VoidParagraph>>,
        check_anchors: bool,
        callback: F,
    ) -> Self {
        HyperlinkEmitter {
            extractor: LinkExtractor {
                paragraph_walker: P::new(),
                arena,
                document,
                link_buf,
                scratch,
                in_paragraph: false,
                check_anchors,
                callback,
            },
            buffers,
            current_tag_is_closing: false,
        }
    }

    fn flush_old_attribute(&mut self) {
        let value = self.buffers.current_attribute_value.as_slice();

        match (
            self.buffers.current_tag_name.as_slice(),
            self.buffers.current_attribute_name.as_slice(),
        ) {
            (b"link" | b"area" | b"a", b"href") => self.extractor.extract_used_link(value),
            (b"a", b"name") => self.extractor.extract_anchor_def(value),
            (b"img" | b"script" | b"iframe", b"src") => self.extractor.extract_used_link(value),
            (b"img", b"srcset") => self.extractor.extract_used_link_srcset(value),
            (b"object", b"data") => self.extractor.extract_used_link(value),
            (_, b"id") => self.extractor.extract_anchor_def(value),
            _ => (),
        }

        self.buffers.current_attribute_name.clear();
        self.buffers.current_attribute_value.clear();
    }
}

impl<'a, P, F> Emitter for HyperlinkEmitter<'a, '_, P, F>
where
    P: ParagraphWalker,
    F: FnMut(Link<'a, P::Paragraph>),
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
        if !P::is_noop() && self.extractor.in_paragraph {
            self.extractor.paragraph_walker.update(c);
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

        let is_paragraph_tag = !P::is_noop() && is_paragraph_tag(&self.buffers.current_tag_name);

        if !self.current_tag_is_closing {
            self.buffers
                .last_start_tag
                .extend(&self.buffers.current_tag_name);

            if is_paragraph_tag {
                self.extractor.begin_paragraph();
            }
        } else if is_paragraph_tag {
            self.extractor.end_paragraph();
        }

        self.buffers.current_tag_name.clear();
        html5gum::naive_next_state(&self.buffers.last_start_tag)
    }

    fn set_self_closing(&mut self) {
        // self-closing flag carries no semantic meaning, so we have to ignore it in order to be
        // correct. we need to dispatch based on tag name, if anything.
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

    fn emit_eof(&mut self) {
        // a paragraph tag may have been left unclosed at eof
        self.extractor.end_paragraph();
    }

    fn emit_current_comment(&mut self) {}
    fn emit_current_doctype(&mut self) {}
    fn emit_error(&mut self, _: Error) {}
    #[inline]
    fn should_emit_errors(&mut self) -> bool {
        false
    }
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
