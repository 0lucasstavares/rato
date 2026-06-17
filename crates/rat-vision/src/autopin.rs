//! Regex pre-filter for automatic pin classification.
//!
//! Scans OCR-delta text for patterns that indicate a noteworthy event
//! (crash, error, test failure, etc.) and returns a short human-readable
//! reason string, or `None` if no pattern matches.

use regex::Regex;
use std::sync::LazyLock;

/// `(pattern_str, reason_str)` ordered table; first match wins.
static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (Regex::new(r"panicked at").unwrap(), "rust panic"),
        (
            Regex::new(r"Traceback \(most recent call last\)").unwrap(),
            "python traceback",
        ),
        (Regex::new(r"error\[E\d+\]").unwrap(), "rustc error"),
        (Regex::new(r"Exception").unwrap(), "exception"),
        (Regex::new(r"\bFAILED\b").unwrap(), "test failure"),
        (Regex::new(r"Segmentation fault").unwrap(), "segfault"),
    ]
});

/// Scan `ocr_delta` for crash/error patterns.
///
/// Returns `Some(reason)` (e.g. `"rust panic"`) for the first match, or
/// `None` if the text looks benign.
pub fn autopin_reason(ocr_delta: &str) -> Option<String> {
    for (re, reason) in PATTERNS.iter() {
        if re.is_match(ocr_delta) {
            return Some((*reason).to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Case {
        input: &'static str,
        expected: Option<&'static str>,
    }

    impl Case {
        fn hit(input: &'static str, reason: &'static str) -> Self {
            Self {
                input,
                expected: Some(reason),
            }
        }
        fn miss(input: &'static str) -> Self {
            Self {
                input,
                expected: None,
            }
        }
    }

    #[test]
    fn autopin_table() {
        let cases = [
            Case::hit(
                "thread 'main' panicked at 'index out of bounds'",
                "rust panic",
            ),
            Case::hit(
                "Traceback (most recent call last):\n  File \"foo.py\"",
                "python traceback",
            ),
            Case::hit("error[E0502]: cannot borrow `x`", "rustc error"),
            Case::hit(
                "java.lang.NullPointerException\nException in thread main",
                "exception",
            ),
            Case::hit("test test_foo ... FAILED", "test failure"),
            Case::hit("Segmentation fault (core dumped)", "segfault"),
            Case::miss("Everything looks good"),
            Case::miss("build succeeded in 1.23s"),
            Case::miss("warning: unused variable `x`"),
        ];

        for c in &cases {
            let got = autopin_reason(c.input);
            assert_eq!(got.as_deref(), c.expected, "input: {:?}", c.input);
        }
    }

    #[test]
    fn empty_text_returns_none() {
        assert_eq!(autopin_reason(""), None);
    }
}
