//! OCR engine trait and fake/null implementations.

use std::cell::Cell;

use crate::screen::{Frame, SourceHealth};

/// A single block of recognised text with its bounding box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrBlock {
    pub text: String,
    /// Bounding box as `(x, y, width, height)` in pixels.
    pub bbox: (u32, u32, u32, u32),
}

/// Runs OCR on a `Frame`.
pub trait OcrEngine {
    fn recognize(&self, frame: &Frame) -> Vec<OcrBlock>;
    fn health(&self) -> SourceHealth;
}

// ── NullOcr ──────────────────────────────────────────────────────────────────

/// No-op OCR engine used in the default build (no `ocr` feature).
pub struct NullOcr;

impl OcrEngine for NullOcr {
    fn recognize(&self, _frame: &Frame) -> Vec<OcrBlock> {
        vec![]
    }

    fn health(&self) -> SourceHealth {
        SourceHealth::Unavailable("ocr feature not built".to_string())
    }
}

// ── FakeOcr ──────────────────────────────────────────────────────────────────

/// A scripted OCR engine for tests.
///
/// Each call to `recognize` returns the next scripted block-list.  When the
/// list is exhausted the last entry is repeated.  If the list is empty an empty
/// `Vec` is returned.
///
/// `recognize` takes `&self`, so interior mutability (`Cell`) is used for the
/// index.
pub struct FakeOcr {
    scripted: Vec<Vec<OcrBlock>>,
    idx: Cell<usize>,
}

impl FakeOcr {
    pub fn new(scripted: Vec<Vec<OcrBlock>>) -> Self {
        Self { scripted, idx: Cell::new(0) }
    }
}

impl OcrEngine for FakeOcr {
    fn recognize(&self, _frame: &Frame) -> Vec<OcrBlock> {
        if self.scripted.is_empty() {
            return vec![];
        }
        let i = self.idx.get();
        let result = self.scripted[i].clone();
        // Advance, clamped at the last element.
        if i + 1 < self.scripted.len() {
            self.idx.set(i + 1);
        }
        result
    }

    fn health(&self) -> SourceHealth {
        SourceHealth::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Frame;

    fn dummy_frame() -> Frame {
        Frame {
            rgba: vec![0u8; 4],
            width: 1,
            height: 1,
            window_title: None,
            captured_ms: 0,
        }
    }

    fn block(text: &str) -> OcrBlock {
        OcrBlock { text: text.to_string(), bbox: (0, 0, 100, 20) }
    }

    #[test]
    fn null_ocr_returns_empty_and_unavailable() {
        let engine = NullOcr;
        let f = dummy_frame();
        assert!(engine.recognize(&f).is_empty());
        assert_eq!(engine.health(), SourceHealth::Unavailable("ocr feature not built".to_string()));
    }

    #[test]
    fn fake_ocr_advances_and_repeats_last() {
        let engine = FakeOcr::new(vec![
            vec![block("line 1")],
            vec![block("line 2"), block("line 3")],
        ]);
        let f = dummy_frame();

        let r0 = engine.recognize(&f);
        assert_eq!(r0.len(), 1);
        assert_eq!(r0[0].text, "line 1");

        let r1 = engine.recognize(&f);
        assert_eq!(r1.len(), 2);
        assert_eq!(r1[0].text, "line 2");

        // Exhausted — repeats last
        let r2 = engine.recognize(&f);
        assert_eq!(r2.len(), 2);
        assert_eq!(r2[0].text, "line 2");
    }

    #[test]
    fn fake_ocr_empty_returns_empty() {
        let engine = FakeOcr::new(vec![]);
        assert!(engine.recognize(&dummy_frame()).is_empty());
    }
}
