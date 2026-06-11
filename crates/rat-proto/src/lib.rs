use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bump on any wire-incompatible change. Checked in the `hello` handshake.
pub const PROTO_VERSION: u32 = 1;

pub mod methods {
    pub const HELLO: &str = "hello";
    pub const STATUS: &str = "status";
    pub const EVENTS_APPEND: &str = "events.append";
    pub const EVENTS_RECENT: &str = "events.recent";
    pub const OBSERVATIONS_RECENT: &str = "observations.recent";
    pub const PROJECTS_LIST: &str = "projects.list";
    pub const SESSIONS_RECENT: &str = "sessions.recent";
    pub const MODE_GET: &str = "mode.get";
}

pub mod errcodes {
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INTERNAL: i64 = -32000;
    pub const HELLO_REQUIRED: i64 = -32001;
    pub const PROTO_MISMATCH: i64 = -32002;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { id, result: Some(result), error: None }
    }

    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self { id, result: None, error: Some(RpcError { code, message: message.into() }) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelloParams {
    pub proto_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelloResult {
    pub proto_version: u32,
    pub server_version: String,
}

/// Client-supplied half of an event; the store assigns `id` and `ts`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NewEvent {
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub ts: i64,
    pub kind: String,
    pub source: String,
    pub project_id: Option<String>,
    pub session_id: Option<String>,
    pub payload: Value,
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusResult {
    pub version: String,
    pub proto_version: u32,
    pub uptime_ms: i64,
    pub event_count: u64,
    pub db_path: String,
}

fn default_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentParams {
    #[serde(default = "default_limit")]
    pub limit: u32,
}

impl Default for RecentParams {
    fn default() -> Self {
        Self { limit: default_limit() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub root_path: String,
    pub name: String,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkSession {
    pub id: String,
    pub project_id: String,
    pub started: i64,
    pub last_activity: i64,
    pub ended: Option<i64>,
    pub commands: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Observation {
    pub id: String,
    pub event_id: Option<String>,
    pub ts: i64,
    pub kind: String,
    pub project_id: Option<String>,
    pub content: String,
    pub meta: Value,
}

/// Client/deriver-supplied half of an observation; the store assigns `id` and `ts`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NewObservation {
    #[serde(default)]
    pub event_id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub content: String,
    #[serde(default)]
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeState {
    /// "active" | "away"
    pub mode: String,
    pub since_ms: i64,
    pub idle_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObsRecentParams {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub kind: Option<String>,
}

impl Default for ObsRecentParams {
    fn default() -> Self {
        Self { limit: default_limit(), kind: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn obs_recent_params_defaults() {
        let p: ObsRecentParams = serde_json::from_str("{}").unwrap();
        assert_eq!(p.limit, 50);
        assert_eq!(p.kind, None);
    }

    #[test]
    fn request_round_trips_and_defaults_params() {
        let r: Request = serde_json::from_str(r#"{"id":1,"method":"status"}"#).unwrap();
        assert_eq!(r.params, Value::Null);
        let s = serde_json::to_string(&r).unwrap();
        let r2: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn response_ok_omits_error_field() {
        let s = serde_json::to_string(&Response::ok(7, json!({"a":1}))).unwrap();
        assert!(!s.contains("error"));
        assert!(s.contains("\"id\":7"));
    }

    #[test]
    fn response_err_omits_result_field() {
        let s =
            serde_json::to_string(&Response::err(7, errcodes::HELLO_REQUIRED, "hello required"))
                .unwrap();
        assert!(!s.contains("result"));
        assert!(s.contains("-32001"));
    }

    #[test]
    fn new_event_minimal_json_parses() {
        let e: NewEvent = serde_json::from_str(r#"{"kind":"k","source":"s"}"#).unwrap();
        assert_eq!(e.payload, Value::Null);
        assert_eq!(e.project_id, None);
    }

    #[test]
    fn recent_params_default_limit_is_50() {
        let p: RecentParams = serde_json::from_str("{}").unwrap();
        assert_eq!(p.limit, 50);
    }
}
