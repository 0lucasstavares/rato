//! Screen source trait and fake implementation.

/// A captured frame of RGBA pixel data.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Raw RGBA bytes (length == width * height * 4).
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Title of the focused window at capture time, if available.
    pub window_title: Option<String>,
    /// Unix timestamp in milliseconds when this frame was captured.
    pub captured_ms: i64,
}

/// Result of a single grab attempt.
#[derive(Debug, Clone)]
pub enum ScreenResult {
    /// A successfully captured frame.
    Frame(Frame),
    /// The source is currently unavailable (e.g. no portal consent yet).
    Unavailable,
}

/// Health of a screen source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceHealth {
    Ok,
    Unavailable(String),
}

/// A source of screen frames.
pub trait ScreenSource {
    /// Attempt to grab the next frame.
    fn grab(&mut self) -> ScreenResult;
    /// Report whether the source is healthy.
    fn health(&self) -> SourceHealth;
}

// ── FakeScreenSource ─────────────────────────────────────────────────────────

/// A scripted screen source for tests.
///
/// `grab()` returns successive results from the `scripted` list.  When the list
/// is exhausted the last result is repeated indefinitely.  If the list is empty
/// `Unavailable` is returned.
pub struct FakeScreenSource {
    scripted: Vec<ScreenResult>,
    idx: usize,
}

impl FakeScreenSource {
    pub fn new(scripted: Vec<ScreenResult>) -> Self {
        Self { scripted, idx: 0 }
    }
}

impl ScreenSource for FakeScreenSource {
    fn grab(&mut self) -> ScreenResult {
        if self.scripted.is_empty() {
            return ScreenResult::Unavailable;
        }
        let result = self.scripted[self.idx].clone();
        // Advance, but clamp at the last element so it repeats.
        if self.idx + 1 < self.scripted.len() {
            self.idx += 1;
        }
        result
    }

    fn health(&self) -> SourceHealth {
        if self.scripted.is_empty() {
            SourceHealth::Unavailable("no scripted frames".to_string())
        } else {
            SourceHealth::Ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(captured_ms: i64) -> Frame {
        Frame {
            rgba: vec![0u8; 4],
            width: 1,
            height: 1,
            window_title: None,
            captured_ms,
        }
    }

    #[test]
    fn fake_source_repeats_last_entry() {
        let frames = vec![
            ScreenResult::Frame(make_frame(1000)),
            ScreenResult::Frame(make_frame(2000)),
        ];
        let mut src = FakeScreenSource::new(frames);

        let r0 = src.grab();
        assert!(matches!(r0, ScreenResult::Frame(f) if f.captured_ms == 1000));
        let r1 = src.grab();
        assert!(matches!(r1, ScreenResult::Frame(f) if f.captured_ms == 2000));
        // Exhausted — last entry repeats
        let r2 = src.grab();
        assert!(matches!(r2, ScreenResult::Frame(f) if f.captured_ms == 2000));
    }

    #[test]
    fn fake_source_empty_returns_unavailable() {
        let mut src = FakeScreenSource::new(vec![]);
        assert!(matches!(src.grab(), ScreenResult::Unavailable));
        assert_eq!(src.health(), SourceHealth::Unavailable("no scripted frames".to_string()));
    }
}
