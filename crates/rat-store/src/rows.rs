use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    pub r#type: String,
    pub project_id: Option<String>,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    pub created: i64,
    pub updated: i64,
    /// JSON array of event-id strings.
    pub source_event_ids: Value,
    pub archived: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NewMemory {
    pub r#type: String,
    pub project_id: Option<String>,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    /// JSON array of event-id strings.
    pub source_event_ids: Value,
}

// ---------------------------------------------------------------------------
// Memory list filter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MemoryFilter {
    pub r#type: Option<String>,
    pub project_id: Option<String>,
    pub include_archived: bool,
}

// ---------------------------------------------------------------------------
// Pushback
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pushback {
    pub id: String,
    pub ts: i64,
    pub mode: String,
    pub trigger: String,
    pub severity: String,
    pub title: String,
    pub message_en: String,
    pub message_pt: String,
    /// JSON array of evidence objects.
    pub evidence: Value,
    /// JSON array of proposed-action objects.
    pub proposals: Value,
    pub confidence: f64,
    pub status: String,
    pub decided_at: Option<i64>,
    pub latency_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewPushback {
    pub mode: String,
    pub trigger: String,
    pub severity: String,
    pub title: String,
    pub message_en: String,
    pub message_pt: String,
    /// JSON value (array of evidence objects).
    pub evidence: Value,
    /// JSON value (array of proposed-action objects).
    pub proposals: Value,
    pub confidence: f64,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Embedding helpers (Vec<f32> ↔ little-endian bytes)
// ---------------------------------------------------------------------------

/// Encode a `Vec<f32>` as a little-endian byte blob.
pub(crate) fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Decode a little-endian byte blob back into a `Vec<f32>`.
pub(crate) fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_round_trip() {
        let original: Vec<f32> = vec![1.0, -0.5, 0.0, 3.14159, f32::MAX, f32::MIN_POSITIVE];
        let encoded = encode_embedding(&original);
        assert_eq!(encoded.len(), original.len() * 4);
        let decoded = decode_embedding(&encoded);
        assert_eq!(original, decoded);
    }

    #[test]
    fn embedding_empty_round_trip() {
        let original: Vec<f32> = vec![];
        let encoded = encode_embedding(&original);
        assert!(encoded.is_empty());
        let decoded = decode_embedding(&encoded);
        assert!(decoded.is_empty());
    }
}
