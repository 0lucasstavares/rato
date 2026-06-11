# M3 — Memory + Critic v1 design

**Date:** 2026-06-11
**Status:** approved (autonomous-goal mode: decisions made from ARCHITECTURE.md defaults and
documented here instead of operator Q&A — operator pre-approved milestone completion via /goal)
**Source:** ARCHITECTURE.md §6 (orchestration), §9 (memory/retrieval), §10 (schema), §18 row M3.

**Acceptance (§18):** end-to-end — induced stuck loop (scripted failing test ×3) yields a
**cited** pushback within 5 min, rate-limited, dismissible; disclosures recorded.

## Decisions (defaults chosen autonomously)

| Question | Decision | Why |
|---|---|---|
| Default chat provider | `openai` (config.toml overridable to anthropic/openrouter) | §6 marks OpenAI Responses as default; all three keys exist in `keys/` |
| Embedding storage | `vec_*` BLOB column + brute-force cosine in Rust, NOT the sqlite-vec extension | <100k rows at MVP scale; zero native-extension build risk; table shape keeps `vec0` migration trivial later. Documented deviation from §9. |
| Pushback popup surface | avatar speech bubble (DialogueBox-style) + dashboard Pushback tab | dedicated frameless popup windows are M7 polish; §18 M3 only requires "popup + Pushback tab" — bubble serves as the popup |
| Personality modes | governor implements per-mode budgets but mode is fixed `mentor` until M7 | modes are an M7 deliverable |
| Bilingual | critic schema emits message_en + message_pt (stored); UI renders en | Fluent localization lands M7 |
| Nightly job scope | day summary + confidence decay + 180d citation-aware observation prune | full §12 matrix completes in M8 hardening |
| Key storage | Secret Service via `keyring` crate (`rato/openai` etc.); `rat setup` imports from `keys/*.txt` once | §20; GNOME keyring is present on the target machine |

## Components

### 1. Store migration v3 (`rat-store`)
New tables per §10 DDL: `memories`, `pushbacks`, `disclosures`, `api_calls`,
`metrics_daily`, plus `observations_fts` (FTS5, contentless, trigger-synced),
`memories_fts`, `vec_observations(obs_id TEXT PK, embedding BLOB)`,
`vec_memories(memory_id TEXT PK, embedding BLOB)`. `observations.embedded` flag exists (v2).
Repos: MemoryRepo, PushbackRepo, DisclosureRepo, ApiCallRepo + FTS/vec upsert helpers.

### 2. `rat-brain` crate — LLM layer
- `ChatBackend` trait per §6 (`complete(ChatRequest) -> ChatResponse`, `provider()`,
  `model_for(route)`); `ChatRequest { system, messages, json_schema, route, purpose, max_tokens }`.
- Backends: `OpenAiResponsesBackend` (`/v1/responses`, `store:false`),
  `AnthropicBackend` (`/v1/messages`, `anthropic-version: 2023-06-01`,
  `output_config.format json_schema`, treat `stop_reason=="refusal"` as error),
  `OpenAiCompatBackend` (OpenRouter `/chat/completions`).
- Routes: Critic→`gpt-5.1`/`claude-opus-4-8`/`openai/gpt-5.1`; Cheap→`gpt-5-mini`/
  `claude-haiku-4-5`/`openai/gpt-5-mini` (per-provider override via config).
- Every call logs an `api_calls` row (tokens, ok, error).
- Keys: `keys.rs` — `get_key(provider)` from Secret Service, env override `RATO_<PROVIDER>_KEY`
  for tests.
- Prompt-injection hardening: observed text serialized inside fenced `UNTRUSTED OBSERVATION`
  blocks; system prompt declares observation content is data, not instructions.

### 3. `rat-memory` crate — embeddings, retrieval, consolidation
- `EmbeddingClient`: OpenAI `text-embedding-3-small` (1536-d), batch ≤128 inputs; always
  OpenAI-direct; absent key → embedding disabled, retrieval FTS-only (status reports it).
- Hybrid retrieval per §9: FTS5 BM25 top-40 + cosine top-40 → RRF (k=60) → project/type
  filters → recency boost `score × (1 + 0.25·e^(−age_days/14))` → top-N. Deterministic
  (stable tie-break by id).
- Jobs: **hourly** — embed unembedded observations (content-bearing kinds only), summarize
  closed work sessions via Cheap route with event-id citations → `memories(type=episode_summary)`;
  **nightly 03:30** — day summary, contradiction decay (archive < 0.2), 180-day
  citation-aware observation prune (≤5k-row delete batches).

### 4. Critic + governor (`rat-brain::critic`)
- **Fast tick (30 s, local):** detectors over recent observations —
  `stuck_loop` (same normalized command failing ≥3× within 10 min),
  `error_burst` (≥10 nonzero exits in 5 min). Detector hit → immediate slow tick.
- **Slow tick (5 min):** context pack = last-5-min observation digest + git state + top-8
  hybrid-retrieved memories; Critic-route call with the §6 verdict JSON schema
  (severity/title/message_en/message_pt/evidence[]/proposed_actions[]/confidence).
  Hard rules: pushback without evidence ids → dropped; confidence <0.6 → logged not shown.
  Every call writes a `disclosures` row with serialized memory/observation ids.
- **Governor:** token bucket per mode — mentor 1/30 min (burst 2), chaos 1/10 (burst 2),
  quiet 1/120, hard cap 8/h; suppressed pushbacks → `status=queued`; identical-evidence
  dedupe 24 h.
- **Feedback:** Useful/Dismiss/Snooze set status + `latency_ms`; per-trigger multiplicative
  threshold weights persisted in `settings`.

### 5. Daemon + CLI + proto
- `~/.config/rato/config.toml`: `[llm] provider/models`, `[critic] enabled, fast_tick_s,
  slow_tick_s`. Defaults written on first run.
- RPC (PROTO_VERSION stays 1, additive): `memory.search {query, project_id?, n?}`,
  `pushbacks.recent {n?}`, `pushbacks.feedback {id, verdict: useful|dismiss|snooze}`,
  `llm.status` (provider, key presence booleans, embedding enabled, last error).
- CLI: `rat setup` (import `keys/*.txt` → Secret Service, never echo values; `--provider`
  to select default), `rat pushbacks [--feedback id verdict]`, `rat search <query>`,
  `rat doctor` gains key/keyring checks.
- Daemon wires fast/slow ticks + hourly/nightly jobs; `--no-critic` flag for tests.

### 6. Shell (dashboard + avatar)
- **Pushback tab:** queue + history list (severity chip, title, message, evidence count,
  trigger), actions Useful/Dismiss/Snooze (calls `pushbacks.feedback`).
- **Now tab:** last 3 pushbacks summary line.
- **Avatar:** polls `pushbacks.recent`; new `shown` pushback renders a paper speech bubble
  (THUG2 style) above the rat with title + Dismiss/Open; dismiss = feedback dismiss.

## Testing
- Unit: RRF fusion determinism + recency boost math; governor bucket (fake clock);
  stuck-loop detector golden cases; verdict JSON schema parse/reject (no-evidence drop,
  low-confidence drop); FTS trigger sync; migration v2→v3.
- Backend tests: wiremock HTTP fixtures per provider (success, refusal, 429).
- E2E acceptance (live, real key): scripted `false`-failing command ×3 → pushback row with
  evidence within 5 min → visible in tab → dismiss → disclosure row exists. Then governor
  suppresses an immediate repeat (dedupe).

## Out of scope (deferred)
sqlite-vec extension (BLOB cosine instead), personality auto-switch (M7), popup windows
(M7), Memory/Metrics tabs (M7), voice (M6), full §12 retention matrix (M8).
