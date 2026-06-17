// Mirrors of rat-proto types (keep in sync with crates/rat-proto/src/lib.rs)

export interface StatusResult {
  version: string;
  proto_version: number;
  uptime_ms: number;
  event_count: number;
  db_path: string;
  sensors: SensorHealthDto[];
}

export interface SensorHealthDto {
  name: string;
  state: "ok" | "unavailable" | string;
  reason?: string | null;
}

export interface RetentionStatusDto {
  last_run_ms: number;
  observations_deleted: number;
  pins_expired: number;
  api_calls_deleted: number;
}

export interface RingMediaStatusDto {
  media: string;
  segment_count: number;
  newest_ms: number | null;
  oldest_ms: number | null;
  ttl_secs: number;
}

export interface VoiceBackendStatusDto {
  name: string;
  state: "ok" | "unavailable" | string;
  reason?: string | null;
}

export interface VoiceStatusDto {
  enabled: boolean;
  backends: VoiceBackendStatusDto[];
  prewake_ring_secs: number;
}

export interface VoiceUtteranceDto {
  id: string;
  ts: number;
  lang: string;
  text: string;
  intent: string | null;
  wake_word: string;
  handled: boolean;
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

export interface MemoryDto {
  id: string;
  type: string;
  project_id: string | null;
  title: string;
  body: string;
  confidence: number;
  created: number;
  updated: number;
  source_event_ids: unknown;
  archived: boolean;
}

export interface DisclosureDto {
  id: string;
  ts: number;
  api_call_id: string | null;
  model: string;
  purpose: string;
  memory_ids: unknown;
  observation_ids: unknown;
}

export interface HitDto {
  id: string;
  kind: "observation" | "memory" | string;
  score: number;
}

export interface ModeState {
  mode: "active" | "away" | string;
  since_ms: number;
  idle_ms: number | null;
}

/** Wire DTO mirroring rat-proto AgentRunDto / rat_store::rows::AgentRun */
export interface AgentRunDto {
  id: string;
  adapter: string;
  task_title: string;
  project_id: string;
  worktree_path: string;
  branch: string;
  tmux_target: string | null;
  mode: string;
  /** "running" | "done" | "failed" | "merged" */
  status: string;
  tokens: unknown;
  cost_usd: number;
  started: number;
  ended: number | null;
  result_summary: string | null;
  diffstat: unknown | null;
}

/** Wire DTO mirroring rat-proto ApprovalDto / rat_store::rows::Approval */
export interface ApprovalDto {
  id: string;
  created: number;
  kind: string;
  risk: number;
  title: string;
  reason: string;
  cwd: string | null;
  target: string | null;
  agent_identity: string;
  payload: unknown;
  expected_impact: unknown;
  expires_at: number;
  /** "pending" | "approved" | "denied" | "expired" | "cancelled" */
  status: string;
  decided_at: number | null;
  decided_via: string | null;
  decision_note: string | null;
  execution: unknown | null;
  spoken_slug: string;
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

/** Wire DTO mirroring rat-proto PinDto / rat_store::rows::Pin */
export interface PinDto {
  id: string;
  kind: string;
  media: string;
  path: string;
  created: number;
  expires_at: number | null;
  reason: string;
  meta: Record<string, unknown>;
}

export interface TerminalDto {
  id: string;
  tty: string;
  pid: number;
  emulator: string;
  tmux_target: string | null;
  role: "operator" | "workbench" | "foreign" | "ignored" | string;
  project_id: string | null;
  cmd_hash: string;
  first_seen: number;
  last_seen: number;
  meta: Record<string, unknown>;
}

export interface DotfileEditDto {
  id: string;
  path: string;
  kind: "json" | "jsonc" | "toml" | "yaml" | "text" | string;
  before_blob: string;
  after_blob: string;
  diff: string;
  reason: string;
  source: string;
  risk: number;
  created: number;
  applied: boolean;
  reverted_by: string | null;
  meta: Record<string, unknown>;
}
