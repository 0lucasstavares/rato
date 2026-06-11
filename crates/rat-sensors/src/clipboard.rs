use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Duration;

use rat_proto::NewEvent;
use regex::Regex;
use serde_json::json;

const MAX_STORED_CHARS: usize = 4096;
const MAX_CONSIDERED_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Text,
    SecretLike,
}

fn secret_patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\bgh[pousr]_[A-Za-z0-9]{36,}\b",
            r"\bsk-[A-Za-z0-9_-]{20,}\b",
            r#"(?i)(api[_-]?key|secret|token|passwd|password)["']?\s*[:=]"#,
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid regex"))
        .collect()
    })
}

/// Pure secret-likeness check — secret-like clipboard content is never stored raw.
pub fn classify(text: &str) -> Classification {
    if secret_patterns().iter().any(|re| re.is_match(text)) {
        Classification::SecretLike
    } else {
        Classification::Text
    }
}

/// Build the event for a fresh clipboard text, or None if it should be skipped.
pub fn event_for(text: &str) -> Option<NewEvent> {
    if text.is_empty() || text.len() > MAX_CONSIDERED_BYTES {
        return None;
    }
    Some(match classify(text) {
        Classification::SecretLike => NewEvent {
            kind: "clipboard_redacted".into(),
            source: "clipboard".into(),
            payload: json!({"text": "[redacted: secret-like]", "len": text.len()}),
            ..Default::default()
        },
        Classification::Text => {
            let truncated = text.len() > MAX_STORED_CHARS;
            let stored: String = if truncated {
                let mut end = MAX_STORED_CHARS;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text[..end].to_string()
            } else {
                text.to_string()
            };
            NewEvent {
                kind: "clipboard_text".into(),
                source: "clipboard".into(),
                payload: json!({"text": stored, "len": text.len(), "truncated": truncated}),
                ..Default::default()
            }
        }
    })
}

/// Poll the system clipboard once a second on a dedicated thread (arboard is
/// blocking). Wayland data-control first; falls back to X11/XWayland.
pub fn spawn(tx: tokio::sync::mpsc::Sender<NewEvent>) {
    std::thread::Builder::new()
        .name("rat-clipboard".into())
        .spawn(move || {
            let mut clipboard = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("clipboard sensor unavailable: {e}");
                    return;
                }
            };
            let mut last_hash: Option<u64> = None;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let Ok(text) = clipboard.get_text() else { continue };
                let mut hasher = DefaultHasher::new();
                text.hash(&mut hasher);
                let h = hasher.finish();
                if last_hash == Some(h) {
                    continue;
                }
                last_hash = Some(h);
                if let Some(ev) = event_for(&text) {
                    if tx.blocking_send(ev).is_err() {
                        return; // daemon shutting down
                    }
                }
            }
        })
        .expect("spawn clipboard thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_classified_and_redacted() {
        let positives = [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc",
            "key AKIAIOSFODNN7EXAMPLE inside",
            "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB",
            "sk-proj-abcdefghijklmnopqrstuvwx",
            "export API_KEY=topsecret",
            "password: hunter2",
        ];
        for p in positives {
            assert_eq!(classify(p), Classification::SecretLike, "should be secret: {p}");
            let ev = event_for(p).unwrap();
            assert_eq!(ev.kind, "clipboard_redacted");
            assert_eq!(ev.payload["text"], "[redacted: secret-like]");
            assert!(!ev.payload.to_string().contains("hunter2"));
        }
    }

    #[test]
    fn normal_text_is_kept() {
        let negatives = [
            "let x = compute(y);",
            "Meeting notes: discuss the keyboard layout",
            "https://example.com/docs",
            "o rato roeu a roupa",
        ];
        for n in negatives {
            assert_eq!(classify(n), Classification::Text, "should be text: {n}");
            let ev = event_for(n).unwrap();
            assert_eq!(ev.kind, "clipboard_text");
            assert_eq!(ev.payload["text"], *n);
            assert_eq!(ev.payload["truncated"], false);
        }
    }

    #[test]
    fn long_text_is_truncated_and_huge_text_skipped() {
        let long = "x".repeat(10_000);
        let ev = event_for(&long).unwrap();
        assert_eq!(ev.payload["truncated"], true);
        assert_eq!(ev.payload["text"].as_str().unwrap().len(), 4096);
        assert_eq!(ev.payload["len"], 10_000);

        let huge = "y".repeat(40 * 1024);
        assert!(event_for(&huge).is_none());
        assert!(event_for("").is_none());
    }
}
