use std::fmt;
use std::hash::Hash;
use std::mem;

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Paragraph {
    hash: [u8; 32],
}

pub struct ParagraphHasher {
    hasher: blake3::Hasher,
}

pub trait ParagraphWalker {
    type Paragraph: Clone + Eq + PartialEq + Hash + Ord + PartialOrd;

    fn new() -> Self;
    fn update_raw(&mut self, text: &str);
    fn finish_paragraph(&mut self) -> Self::Paragraph;

    fn update(&mut self, text: &str) {
        for word in text.trim().split(' ') {
            if !word.is_empty() {
                self.update_raw(word.trim_end());
                self.update_raw(" ");
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

    fn update_raw(&mut self, text: &str) {
        self.hasher.update(text.as_bytes());
    }

    fn finish_paragraph(&mut self) -> Self::Paragraph {
        let rv = Paragraph {
            hash: self.hasher.finalize().into(),
        };
        self.hasher.reset();
        rv
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

    fn update_raw(&mut self, text: &str) {
        self.inner.update(text);
        self.contents.push_str(text);
    }

    fn finish_paragraph(&mut self) -> Self::Paragraph {
        let inner = self.inner.finish_paragraph();
        DebugParagraph {
            inner,
            contents: mem::replace(&mut self.contents, String::new()),
        }
    }
}
