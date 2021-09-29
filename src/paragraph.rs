use std::fmt;
use std::hash::Hash;
use std::mem;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Paragraph {
    hash: [u8; 32],
}

pub struct ParagraphHasher {
    hasher: blake3::Hasher,
}

pub trait ParagraphWalker: Send {
    type Paragraph: Clone + Eq + PartialEq + Hash + Ord + PartialOrd + Send;

    fn new() -> Self;
    fn update_raw(&mut self, text: &[u8]);
    fn finish_paragraph(&mut self) -> Option<Self::Paragraph>;

    fn update(&mut self, text: &[u8]) {
        for c in text {
            if !c.is_ascii_whitespace() {
                self.update_raw(&[*c]);
            }
        }
    }
}

impl ParagraphWalker for ParagraphHasher {
    type Paragraph = Paragraph;

    fn new() -> Self {
        ParagraphHasher {
            hasher: blake3::Hasher::new(),
        }
    }

    fn update_raw(&mut self, text: &[u8]) {
        self.hasher.update(text);
    }

    fn finish_paragraph(&mut self) -> Option<Self::Paragraph> {
        let rv = Paragraph {
            hash: self.hasher.finalize().into(),
        };
        self.hasher.reset();
        Some(rv)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct DebugParagraph<T> {
    inner: T,
    contents: String,
}

impl fmt::Display for DebugParagraph<Paragraph> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.contents)
    }
}

pub struct DebugParagraphWalker<T> {
    inner: T,
    contents: String,
}

impl<T> ParagraphWalker for DebugParagraphWalker<T>
where
    T: ParagraphWalker,
{
    type Paragraph = DebugParagraph<T::Paragraph>;

    fn new() -> Self {
        DebugParagraphWalker {
            inner: T::new(),
            contents: String::new(),
        }
    }

    fn update_raw(&mut self, text: &[u8]) {
        self.inner.update(text);
        self.contents.push_str(&String::from_utf8_lossy(text));
    }

    fn finish_paragraph(&mut self) -> Option<Self::Paragraph> {
        let inner = self.inner.finish_paragraph()?;
        Some(DebugParagraph {
            inner,
            contents: mem::take(&mut self.contents),
        })
    }
}

pub struct NoopParagraphWalker;

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum VoidParagraph {}

impl ParagraphWalker for NoopParagraphWalker {
    type Paragraph = VoidParagraph;

    fn new() -> Self {
        NoopParagraphWalker
    }

    fn update_raw(&mut self, _text: &[u8]) {}

    fn finish_paragraph(&mut self) -> Option<Self::Paragraph> {
        None
    }
}
