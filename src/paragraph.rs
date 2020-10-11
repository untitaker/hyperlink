#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Paragraph {
    hash: [u8; 32],
}

pub struct ParagraphHasher {
    hasher: blake3::Hasher,
}

impl ParagraphHasher {
    pub fn new() -> Self {
        ParagraphHasher {
            hasher: blake3::Hasher::new(),
        }
    }

    pub fn update(&mut self, text: &str) {
        self.hasher.update(text.as_bytes());
    }

    pub fn finish_paragraph(&mut self) -> Paragraph {
        let rv = Paragraph {
            hash: self.hasher.finalize().into(),
        };
        self.hasher.reset();
        rv
    }
}
