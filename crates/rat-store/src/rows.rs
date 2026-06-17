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
    /// JSON array of observation.id values (the ids shown to + cited by the LLM), not events.event_id.
    pub source_event_ids: Value,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Disclosure {
    pub id: String,
    pub ts: i64,
    pub api_call_id: Option<String>,
    pub model: String,
    pub purpose: String,
    pub memory_ids: Value,
    pub observation_ids: Value,
}

#[derive(Debug, Clone, Default)]
pub struct NewMemory {
    pub r#type: String,
    pub project_id: Option<String>,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    /// JSON array of observation.id values (the ids shown to + cited by the LLM), not events.event_id.
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
// VoiceUtterance (v7)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceUtterance {
    pub id: String,
    pub ts: i64,
    pub lang: String,
    pub text: String,
    pub intent: Option<String>,
    pub wake_word: String,
    pub handled: bool,
}

#[derive(Debug, Clone)]
pub struct NewVoiceUtterance {
    pub lang: String,
    pub text: String,
    pub intent: Option<String>,
    pub wake_word: String,
    pub handled: bool,
}

// ---------------------------------------------------------------------------
// Terminal + DotfileEdit (v8)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Terminal {
    pub id: String,
    pub tty: String,
    pub pid: i64,
    pub emulator: String,
    pub tmux_target: Option<String>,
    /// operator | workbench | foreign | ignored
    pub role: String,
    pub project_id: Option<String>,
    pub cmd_hash: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub meta: Value,
}

#[derive(Debug, Clone)]
pub struct NewTerminal {
    pub tty: String,
    pub pid: i64,
    pub emulator: String,
    pub tmux_target: Option<String>,
    pub role: String,
    pub project_id: Option<String>,
    pub cmd_hash: String,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DotfileEdit {
    pub id: String,
    pub path: String,
    pub kind: String,
    pub before_blob: String,
    pub after_blob: String,
    pub diff: String,
    pub reason: String,
    pub source: String,
    pub risk: i64,
    pub created: i64,
    pub applied: bool,
    pub reverted_by: Option<String>,
    pub meta: Value,
}

#[derive(Debug, Clone)]
pub struct NewDotfileEdit {
    pub path: String,
    pub kind: String,
    pub before_blob: String,
    pub after_blob: String,
    pub diff: String,
    pub reason: String,
    pub source: String,
    pub risk: i64,
    pub applied: bool,
    pub meta: Value,
}

// ---------------------------------------------------------------------------
// Approval (v4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Approval {
    pub id: String,
    pub created: i64,
    pub kind: String,
    pub risk: i64,
    pub title: String,
    pub reason: String,
    pub cwd: Option<String>,
    pub target: Option<String>,
    pub agent_identity: String,
    /// JSON payload: exact command/bytes/diff
    pub payload: Value,
    /// JSON: expected impact
    pub expected_impact: Value,
    pub expires_at: i64,
    /// pending | approved | denied | expired | cancelled
    pub status: String,
    pub decided_at: Option<i64>,
    /// popup | dashboard | voice | cli
    pub decided_via: Option<String>,
    pub decision_note: Option<String>,
    /// JSON: started, ended, exit_code, output_ref, verified_target
    pub execution: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct NewApproval {
    pub kind: String,
    pub risk: i64,
    pub title: String,
    pub reason: String,
    pub cwd: Option<String>,
    pub target: Option<String>,
    pub agent_identity: String,
    pub payload: Value,
    pub expected_impact: Value,
    pub expires_at: i64,
}

// ---------------------------------------------------------------------------
// AgentRun (v4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRun {
    pub id: String,
    pub adapter: String,
    pub task_title: String,
    pub project_id: String,
    pub worktree_path: String,
    pub branch: String,
    pub tmux_target: Option<String>,
    /// headless | interactive
    pub mode: String,
    /// running | done | failed | merged (free string)
    pub status: String,
    /// JSON token counts
    pub tokens: Value,
    pub cost_usd: f64,
    pub started: i64,
    pub ended: Option<i64>,
    pub result_summary: Option<String>,
    /// JSON diffstat
    pub diffstat: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct NewAgentRun {
    pub adapter: String,
    pub task_title: String,
    pub project_id: String,
    pub worktree_path: String,
    pub branch: String,
    pub tmux_target: Option<String>,
    pub mode: String,
    pub tokens: Value,
    pub cost_usd: f64,
    pub started: i64,
}

// ---------------------------------------------------------------------------
// Blob (v4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Blob {
    pub id: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
    pub created: i64,
}

// ---------------------------------------------------------------------------
// Pin (v5)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pin {
    pub id: String,
    /// `auto` | `manual`
    pub kind: String,
    /// `screen` | `audio` | `clipboard`
    pub media: String,
    pub path: String,
    pub created: i64,
    pub expires_at: Option<i64>,
    pub reason: String,
    /// Arbitrary JSON metadata.
    pub meta: Value,
}

#[derive(Debug, Clone)]
pub struct NewPin {
    pub kind: String,
    pub media: String,
    pub path: String,
    pub expires_at: Option<i64>,
    pub reason: String,
    pub meta: Value,
}

// ---------------------------------------------------------------------------
// RetentionStatus (v6)
// ---------------------------------------------------------------------------

/// Last-prune snapshot, persisted as a single row (id="last") so it survives
/// daemon restarts. Surfaced via the `retention.status` RPC for the Sensors tab.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetentionStatus {
    pub last_run_ms: i64,
    pub observations_deleted: u32,
    pub pins_expired: u32,
    pub api_calls_deleted: u32,
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
        let original: Vec<f32> = vec![
            1.0,
            -0.5,
            0.0,
            std::f32::consts::PI,
            f32::MAX,
            f32::MIN_POSITIVE,
        ];
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
