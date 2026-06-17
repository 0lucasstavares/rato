//! Capture pipeline: grab → dHash dedup → OCR → delta → JPEG output.

use std::collections::HashSet;

use image::{DynamicImage, GrayImage, ImageBuffer, Rgba};

use crate::dhash::{dhash, hamming};
use crate::ocr::OcrEngine;
use crate::screen::{Frame, ScreenResult, ScreenSource};

/// Output produced by a single successful `tick`.
#[derive(Debug, Clone)]
pub struct CaptureOutput {
    /// JPEG-encoded frame bytes (quality 70).
    pub frame_jpeg: Vec<u8>,
    /// Lines that appeared in the current OCR text but not in the previous
    /// frame's OCR text (empty string if OCR produced nothing new).
    pub ocr_delta: String,
    /// Window title at capture time.
    pub window_title: Option<String>,
    /// Unix timestamp in milliseconds from the captured frame.
    pub captured_ms: i64,
}

/// The main capture pipeline.
///
/// `S` is the screen source, `O` is the OCR engine.  Both are generic so tests
/// can inject `FakeScreenSource` + `FakeOcr` without touching real hardware.
pub struct CapturePipeline<S: ScreenSource, O: OcrEngine> {
    pub source: S,
    pub ocr: O,
    /// Hamming distance threshold for deduplication.  Frames whose dHash is
    /// within this distance of the last kept frame are skipped.
    pub dedup_distance: u32,
    last_hash: Option<u64>,
    last_ocr_text: String,
}

impl<S: ScreenSource, O: OcrEngine> CapturePipeline<S, O> {
    pub fn new(source: S, ocr: O) -> Self {
        Self {
            source,
            ocr,
            dedup_distance: 4,
            last_hash: None,
            last_ocr_text: String::new(),
        }
    }

    /// Run one capture cycle.
    ///
    /// Returns `None` if:
    /// - the source is `Unavailable`, or
    /// - the frame is a near-duplicate of the last kept frame (dHash Hamming
    ///   distance ≤ `dedup_distance`).
    ///
    /// Returns `Some(CaptureOutput)` for every unique frame, including frames
    /// whose `ocr_delta` is empty (the ring still needs to store them).
    pub fn tick(&mut self) -> Option<CaptureOutput> {
        let frame = match self.source.grab() {
            ScreenResult::Frame(f) => f,
            ScreenResult::Unavailable => return None,
        };

        // Convert RGBA → grayscale for hashing.
        let gray = rgba_to_gray(&frame);
        let new_hash = dhash(&gray);

        // Dedup check.
        if let Some(prev_hash) = self.last_hash {
            if hamming(prev_hash, new_hash) <= self.dedup_distance {
                return None; // duplicate frame, skip
            }
        }
        self.last_hash = Some(new_hash);

        // OCR.
        let blocks = self.ocr.recognize(&frame);
        let full_text: String = blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        // Line-set delta: lines present now but not in the previous text.
        let prev_lines: HashSet<&str> = self
            .last_ocr_text
            .lines()
            .filter(|l| !l.is_empty())
            .collect();
        let new_lines: Vec<&str> = full_text
            .lines()
            .filter(|l| !l.is_empty() && !prev_lines.contains(*l))
            .collect();
        let ocr_delta = new_lines.join("\n");
        self.last_ocr_text = full_text;

        // JPEG-encode at quality 70.
        let frame_jpeg = encode_jpeg(&frame, 70);

        Some(CaptureOutput {
            frame_jpeg,
            ocr_delta,
            window_title: frame.window_title.clone(),
            captured_ms: frame.captured_ms,
        })
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Convert an RGBA `Frame` to a `GrayImage` for dHash computation.
fn rgba_to_gray(frame: &Frame) -> GrayImage {
    let pixels = &frame.rgba;
    let w = frame.width;
    let h = frame.height;

    // Build an Rgba image then convert to luma.
    let rgba_img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(w, h, pixels.to_vec())
        .unwrap_or_else(|| ImageBuffer::new(w.max(1), h.max(1)));

    DynamicImage::ImageRgba8(rgba_img).to_luma8()
}

/// JPEG-encode a `Frame` at the given quality (0–100).
fn encode_jpeg(frame: &Frame, quality: u8) -> Vec<u8> {
    let rgba_img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(frame.width, frame.height, frame.rgba.clone())
            .unwrap_or_else(|| ImageBuffer::new(frame.width.max(1), frame.height.max(1)));

    let rgb = DynamicImage::ImageRgba8(rgba_img).to_rgb8();

    let mut buf = Vec::new();
    {
        use image::codecs::jpeg::JpegEncoder;
        let mut enc = JpegEncoder::new_with_quality(&mut buf, quality);
        enc.encode_image(&DynamicImage::ImageRgb8(rgb))
            .expect("JPEG encode failed");
    }
    buf
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{FakeOcr, NullOcr, OcrBlock};
    use crate::screen::{FakeScreenSource, Frame, ScreenResult, SourceHealth};

    fn make_frame(captured_ms: i64, rgba: Vec<u8>, w: u32, h: u32) -> Frame {
        Frame {
            rgba,
            width: w,
            height: h,
            window_title: None,
            captured_ms,
        }
    }

    /// Produce a solid-colour RGBA frame (w×h).
    fn solid_frame(captured_ms: i64, r: u8, g: u8, b: u8, w: u32, h: u32) -> Frame {
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [r, g, b, 255]).collect();
        make_frame(captured_ms, rgba, w, h)
    }

    /// Produce an RGBA frame that grades horizontally from (v,v,v) to (255,255,255).
    fn gradient_frame(captured_ms: i64, w: u32, h: u32) -> Frame {
        let rgba: Vec<u8> = (0..h)
            .flat_map(|_| {
                (0..w).flat_map(|x| {
                    let v = ((x * 255) / w.max(1)) as u8;
                    [v, v, v, 255u8]
                })
            })
            .collect();
        make_frame(captured_ms, rgba, w, h)
    }

    fn block(text: &str) -> OcrBlock {
        OcrBlock {
            text: text.to_string(),
            bbox: (0, 0, 100, 20),
        }
    }

    // ── dedup behaviour ──────────────────────────────────────────────────────

    #[test]
    fn tick_returns_some_for_first_frame() {
        let src = FakeScreenSource::new(vec![ScreenResult::Frame(solid_frame(
            1000, 100, 100, 100, 32, 32,
        ))]);
        let mut pipe = CapturePipeline::new(src, NullOcr);
        assert!(pipe.tick().is_some());
    }

    #[test]
    fn tick_skips_near_duplicate_frame() {
        // Frame A and near-dup of A (single-pixel tweak).
        let mut frame_a_rgba: Vec<u8> = (0..32 * 32).flat_map(|_| [100u8, 100, 100, 255]).collect();
        let frame_a = make_frame(1000, frame_a_rgba.clone(), 32, 32);

        // Near-dup: change the very first pixel by 1 — after 9×8 resize this
        // won't change any comparison bit, so dHash distance should be 0.
        frame_a_rgba[0] = 101;
        let frame_near = make_frame(2000, frame_a_rgba, 32, 32);

        let src = FakeScreenSource::new(vec![
            ScreenResult::Frame(frame_a),
            ScreenResult::Frame(frame_near),
        ]);
        let mut pipe = CapturePipeline::new(src, NullOcr);
        assert!(pipe.tick().is_some(), "first frame should be kept");
        assert!(pipe.tick().is_none(), "near-dup should be skipped");
    }

    #[test]
    fn tick_keeps_distinct_frame() {
        // Frame A: dark solid; Frame B: bright gradient — should be well above distance 4.
        let frame_a = solid_frame(1000, 20, 20, 20, 64, 64);
        let frame_b = gradient_frame(2000, 64, 64);

        let src = FakeScreenSource::new(vec![
            ScreenResult::Frame(frame_a),
            ScreenResult::Frame(frame_b),
        ]);
        let mut pipe = CapturePipeline::new(src, NullOcr);
        let r0 = pipe.tick();
        let r1 = pipe.tick();
        assert!(r0.is_some(), "frame A should be kept");
        assert!(r1.is_some(), "distinct frame B should be kept");
    }

    // ── three-frame scenario (spec: A, near-dup of A, distinct B) ────────────

    #[test]
    fn three_frame_pipeline_with_ocr_delta() {
        // Frame A: dark solid  → OCR: "hello"
        // Near-dup of A        → skipped (no tick call to OCR)
        // Frame B: gradient    → OCR: "hello\nworld"

        let frame_a = solid_frame(1000, 10, 10, 10, 64, 64);

        // Near-dup of A: single corner pixel tweak on same solid colour.
        let mut near_dup_rgba: Vec<u8> = (0..64 * 64).flat_map(|_| [10u8, 10, 10, 255]).collect();
        near_dup_rgba[0] = 11; // imperceptible
        let frame_near = make_frame(2000, near_dup_rgba, 64, 64);

        let frame_b = gradient_frame(3000, 64, 64);

        let src = FakeScreenSource::new(vec![
            ScreenResult::Frame(frame_a),
            ScreenResult::Frame(frame_near),
            ScreenResult::Frame(frame_b),
        ]);

        let ocr = FakeOcr::new(vec![
            vec![block("hello")],
            // no entry for near-dup (tick won't call OCR for it)
            vec![block("hello"), block("world")],
        ]);

        let mut pipe = CapturePipeline::new(src, ocr);

        let out_a = pipe.tick().expect("frame A should be kept");
        assert_eq!(out_a.captured_ms, 1000);
        // "hello" is new (prev text was empty) → delta = "hello"
        assert_eq!(out_a.ocr_delta, "hello");

        let out_near = pipe.tick();
        assert!(out_near.is_none(), "near-dup must be skipped");

        let out_b = pipe.tick().expect("distinct frame B should be kept");
        assert_eq!(out_b.captured_ms, 3000);
        // "hello" was seen before; "world" is new → delta = "world"
        assert_eq!(out_b.ocr_delta, "world");
    }

    // ── Unavailable source ───────────────────────────────────────────────────

    #[test]
    fn unavailable_source_tick_returns_none() {
        let src = FakeScreenSource::new(vec![ScreenResult::Unavailable]);
        let mut pipe = CapturePipeline::new(src, NullOcr);
        assert!(pipe.tick().is_none());
        // Health is surfaced via the source field
        assert_eq!(pipe.source.health(), SourceHealth::Ok); // FakeScreenSource with one entry is Ok
    }

    #[test]
    fn empty_source_health_is_unavailable() {
        let src = FakeScreenSource::new(vec![]);
        let mut pipe = CapturePipeline::new(src, NullOcr);
        assert!(pipe.tick().is_none());
        assert_eq!(
            pipe.source.health(),
            SourceHealth::Unavailable("no scripted frames".to_string())
        );
    }

    // ── JPEG output ──────────────────────────────────────────────────────────

    #[test]
    fn tick_output_contains_valid_jpeg() {
        let src = FakeScreenSource::new(vec![ScreenResult::Frame(gradient_frame(1, 16, 16))]);
        let mut pipe = CapturePipeline::new(src, NullOcr);
        let out = pipe.tick().unwrap();
        // JPEG magic bytes: FF D8 FF
        assert!(
            out.frame_jpeg.starts_with(&[0xFF, 0xD8, 0xFF]),
            "frame_jpeg should start with JPEG magic bytes"
        );
    }
}
