//! Feature-gated Tesseract OCR placeholder.
//!
//! TODO(operator): wire leptess once tesseract/leptonica development packages
//! are present. Until then the feature reports `Unavailable`.

use crate::ocr::{OcrBlock, OcrEngine};
use crate::screen::{Frame, SourceHealth};

pub struct TesseractOcr {
    reason: String,
}

impl TesseractOcr {
    pub fn new() -> Self {
        Self {
            reason: "ocr feature built, but tesseract backend is not wired".to_string(),
        }
    }
}

impl Default for TesseractOcr {
    fn default() -> Self {
        Self::new()
    }
}

impl OcrEngine for TesseractOcr {
    fn recognize(&self, _frame: &Frame) -> Vec<OcrBlock> {
        vec![]
    }

    fn health(&self) -> SourceHealth {
        SourceHealth::Unavailable(self.reason.clone())
    }
}
