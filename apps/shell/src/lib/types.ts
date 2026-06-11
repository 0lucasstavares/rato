// Mirrors of rat-proto types (keep in sync with crates/rat-proto/src/lib.rs)

export interface StatusResult {
  version: string;
  proto_version: number;
  uptime_ms: number;
  event_count: number;
  db_path: string;
}

export interface RatEvent {
  id: string;
  ts: number;
  kind: string;
  source: string;
  project_id: string | null;
  session_id: string | null;
  payload: unknown;
  lang: string | null;
}

export interface Project {
  id: string;
  root_path: string;
  name: string;
  first_seen: number;
  last_seen: number;
}

export interface WorkSession {
  id: string;
  project_id: string;
  started: number;
  last_activity: number;
  ended: number | null;
  commands: number;
}

export interface Observation {
  id: string;
  event_id: string | null;
  ts: number;
  kind: string;
  project_id: string | null;
  content: string;
  meta: Record<string, unknown>;
}

export interface ModeState {
  mode: "active" | "away" | string;
  since_ms: number;
  idle_ms: number | null;
}

/** Wire DTO mirroring rat-proto PushbackDto / rat_store::rows::Pushback */
export interface PushbackDto {
  id: string;
  ts: number;
  mode: string;
  trigger: string;
  severity: "nudge" | "warn" | "block-suggest" | string;
  title: string;
  message_en: string;
  message_pt: string;
  /** JSON array of {observation_id, quote} objects */
  evidence: Array<{ observation_id: string; quote: string }>;
  /** JSON array of {kind, detail} objects */
  proposals: Array<{ kind: string; detail: string }>;
  confidence: number;
  /** "shown" | "queued" | "accepted" | "dismissed" | "snoozed" */
  status: string;
  decided_at: number | null;
  latency_ms: number | null;
}
