# RATO — Architecture Specification v1.0

Single-user, local-first Linux developer companion: a 24/7 daemon with a PS2-style low-poly rat avatar that observes, remembers, critiques, proposes, and — after approval — acts.

Product name **RATO**. Binaries: `ratd` (daemon), `rat` (CLI), `rato-shell` (Tauri avatar + dashboard). This document is implementation-ready: a competent agent should be able to build it without further architectural decisions.

---

## 1. Executive recommendation

Build a **two-process system**:

1. **`ratd`** — a headless Rust/Tokio daemon, run via `systemd --user`, that owns *all* state, sensors, memory, policy, orchestration, and execution. It exposes one Unix-domain-socket NDJSON-RPC API.
2. **`rato-shell`** — a Tauri v2 app (Svelte 5 + TypeScript) with two windows: a transparent always-on-top **avatar overlay** (Three.js, glTF rat) and a **dashboard**. It is a *thin client*: zero business logic, zero direct DB access; it renders daemon state and forwards user intent.

Plus **`rat`** — a clap CLI speaking the same socket protocol (status, modes, approvals, shell hooks).

**Canonical store is a single SQLite database** (WAL mode) with `sqlite-vec` for embeddings and FTS5 for lexical search. The AI workbench is a dedicated tmux server (`tmux -L rato`) where agent CLIs (Claude Code, Codex, Aider, Gemini) run inside **isolated git worktrees**; merges back to the operator's repo always pass the approval flow. Terminal injection into the operator's own terminals is a separate, heavily-gated path (tmux `paste-buffer` → portal RemoteDesktop/libei → ydotool → xdotool, chosen per environment).

**Key deviations from the preferred stack (each strongly justified):**

| Preferred | Chosen instead | Why |
|---|---|---|
| Local Qdrant | **`sqlite-vec` inside the canonical SQLite DB** | Single-user corpus stays well under 1M vectors; brute-force vec0 queries over ≤500k×1536 dims return in tens of ms. Qdrant adds a second 24/7 service, a second backup/consistency domain, and a crash-recovery story for zero retrieval benefit at this scale. Migration path: the `Retriever` trait isolates the backend; swap to Qdrant if the corpus ever exceeds ~1M vectors. |
| openWakeWord *or* Porcupine | **openWakeWord (ONNX via `ort`)** | Porcupine requires a Picovoice license/key for custom keywords; "rato"/"ei rato" are custom. openWakeWord trains custom models from synthetic TTS data, runs <1% CPU, fully local. |
| Three.js *or* PixiJS | **Three.js** | The rat is a 3D low-poly glTF model with skeletal animation; PixiJS is 2D. |

Everything else follows the preferred stack. Rejected wholesale: Electron (RAM, no benefit over Tauri), GNOME Shell extension for the avatar (locks to one DE), Vosk for STT (whisper is strictly better for bilingual pt-BR/en), running agent CLIs via raw PTYs as the primary path (tmux gives persistence, observability, and human attach for free).

---

## 2. Full tech stack table

| Layer | Choice | Version / notes |
|---|---|---|
| Daemon language | Rust | 1.88+, edition 2024, workspace of ~11 crates |
| Async runtime | Tokio | 1.x, multi-threaded; blocking work via dedicated threads |
| CLI parsing | clap | v4, derive |
| Logging | tracing + tracing-subscriber | journald layer when under systemd; `RAT_LOG` env filter |
| IPC | Unix domain socket, NDJSON-RPC 2.0 + pub/sub | `$XDG_RUNTIME_DIR/rato/ratd.sock`, mode 0600; versioned `proto_version` handshake |
| Canonical store | SQLite | rusqlite, WAL, `synchronous=NORMAL`; single-writer actor thread; migrations via `rusqlite_migration` |
| Vector search | sqlite-vec | `vec0` virtual tables, 1536-dim f32 |
| Lexical search | SQLite FTS5 | `unicode61` tokenizer, en+pt |
| Embeddings | OpenAI `text-embedding-3-small` | 1536 dims; batched (≤128 inputs/call) |
| LLM orchestration | **Three providers behind one `ChatBackend` trait**: OpenAI Responses API (default), Anthropic Messages API, OpenRouter (OpenAI-compatible Chat Completions) | provider selected in config; per-route model overrides; structured outputs (JSON schema) on all three; thin custom `reqwest` clients (no SDK churn). Defaults: OpenAI `gpt-5.1`/`gpt-5-mini`; Anthropic `claude-opus-4-8`/`claude-haiku-4-5`; OpenRouter `openai/gpt-5.1`/`openai/gpt-5-mini` (any OpenRouter model id configurable) |
| Embeddings provider | OpenAI direct only | Neither Anthropic nor OpenRouter serve an embeddings endpoint; embeddings always go to `api.openai.com`. Without an OpenAI key, retrieval degrades gracefully to FTS5-only |
| Agent CLIs | Claude Code, Codex CLI, Aider, Gemini CLI | adapter trait; headless modes (`claude -p --output-format stream-json`, `codex exec --json`, `aider --yes-always --message`, `gemini -p`) and interactive tmux panes |
| Workbench | tmux ≥ 3.3 | dedicated server `tmux -L rato`; one control-mode client (`-C`) for the event stream |
| Worktrees | git ≥ 2.40 CLI (shell-out) + `gix` for read paths | worktrees under `~/.local/share/rato/worktrees/` |
| PTY (non-tmux exec) | portable-pty | only for one-shot adapter runs that don't need a pane |
| Screen capture | XDG Desktop Portal ScreenCast → PipeWire | `ashpd` + `pipewire-rs`; persistent restore token |
| OCR | Tesseract 5 via `leptess` | `eng+por` traineddata; preprocessing with `image`/`imageproc` (OpenCV optional cargo feature) |
| Mic capture | PipeWire | 16 kHz mono f32 |
| VAD | Silero VAD (ONNX via `ort`) | endpointing for utterances |
| Wake word | openWakeWord (ONNX via `ort`) | 4 models: rat / hey-rat / rato / ei-rato |
| STT | whisper.cpp via `whisper-rs` | default `small` (multilingual, CPU); auto-upgrade to `large-v3-turbo` Q5 when CUDA/Vulkan detected |
| TTS | Piper | voices `en_US-lessac-medium`, `pt_BR-faber-medium` |
| Clipboard watch | wl-clipboard-rs (`zwlr_data_control`) on Wayland; arboard polling (1 s) on X11 | |
| Idle detection | `ext-idle-notify-v1` (Wayland), XScreenSaver ext (X11), logind `IdleHint` fallback | |
| Process watch | `procfs` crate (+ `sysinfo` for totals) | read-only |
| Shell hooks | bash/zsh/fish snippets sourced by user (`rat shell-init`) | preexec/precmd emit NDJSON to the socket |
| Secrets | keyring-rs → Secret Service (libsecret) | OpenAI key, pin-encryption key |
| Raw-buffer crypto | XChaCha20-Poly1305 (`chacha20poly1305` crate) | ephemeral per-run key, `mlock`ed |
| UI shell | Tauri v2 | two windows: avatar overlay + dashboard |
| UI framework | Svelte 5 + TypeScript + Vite | no Tailwind/shadcn; custom "HUD-PS2" CSS design system |
| Avatar rendering | Three.js | glTF 2.0, skeletal animation, custom PS2 shaders |
| Asset pipeline | Blender 4.x → glTF | ~2,200 tris, 256² texture, 9 animation clips |
| Notifications | own Tauri dialogue-box popups (primary); notify-rust (fallback when shell not running) | |
| Desktop injection | tmux `load-buffer`/`paste-buffer` → portal RemoteDesktop (libei) → ydotool → xdotool | per-environment chain, §8 |
| i18n | Fluent (`fluent-rs` / `@fluent/bundle`) | `en-US`, `pt-BR` bundles |
| Service | systemd --user units | `ratd.service`, `rato-shell.service` (graphical-session.target) |
| Tests | cargo-nextest, insta, proptest; Playwright; tmux integration harness | §19 |
| Task runner / packaging | `just` + install script; optional AUR/`.deb` later | |

---

## 3. Recommended architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│ systemd --user                                                       │
│                                                                      │
│  ┌──────────────────────── ratd (Rust daemon) ─────────────────────┐ │
│  │                                                                  │ │
│  │  Sensor Hub ──► Event Bus (tokio broadcast) ──► SQLite writer   │ │
│  │   screen │ mic │ clipboard │ shell │ proc │ git │ transcripts   │ │
│  │      │                                                          │ │
│  │      ▼                                                          │ │
│  │  Encrypted 20-min ring buffer (raw)   Derivers (OCR, STT,       │ │
│  │                                        classify, embed)         │ │
│  │                                              │                  │ │
│  │  Memory Service ◄────────────────────────────┘                  │ │
│  │   SQLite + FTS5 + sqlite-vec, consolidation jobs                │ │
│  │                                                                  │ │
│  │  Critic/Pushback Engine ──► Interruption Governor               │ │
│  │  Orchestrator (OpenAI Responses) ──► Proposals                  │ │
│  │  Policy Engine ──► Approval Service (durable, SQLite)           │ │
│  │  Workbench (tmux -L rato + worktrees + agent adapters)          │ │
│  │  Injection Service (gated)                                      │ │
│  │  Voice (wake → VAD → whisper → intent)                          │ │
│  │                                                                  │ │
│  │  RPC server: $XDG_RUNTIME_DIR/rato/ratd.sock (NDJSON-RPC+pubsub)│ │
│  └────────────▲──────────────────▲──────────────────▲──────────────┘ │
│               │                  │                  │                │
│        rato-shell (Tauri)     rat (CLI)       shell hooks            │
│        ├ avatar window        approvals,      preexec/precmd         │
│        └ dashboard window     modes, status   emitters               │
└─────────────────────────────────────────────────────────────────────┘
```

**Principles (binding):**

1. **Daemon owns everything.** UI crash/restart loses nothing. The shell can be closed; `ratd` keeps observing (popups fall back to `notify-rust`).
2. **One append-only event spine.** Every sensor reading, decision, approval, action, and API call becomes a row in `events`; all higher-level state is derived and rebuildable.
3. **Policy is code, personality is data.** The `PolicyEngine` (risk tiers, retention, network rules, approval requirements) is a pure module whose config is only writable via Settings UI/CLI with its own audit trail. The personality system consumes policy decisions; it has *no API* to change them (enforced by crate visibility: `rat-personality` has no dependency on `rat-policy`'s write interface).
4. **No covert mode.** Sensor state is part of the avatar's render state, driven by the same flags the sensors themselves read. There is no code path that captures while the indicator says off (single source of truth: `SensorGate` struct; sensors and UI both subscribe to it).
5. **Observed content is untrusted input.** OCR text, transcripts, clipboard, and agent output never become instructions; they are always wrapped as data in prompts (§17).

**IPC protocol** (`rat-proto` crate, shared types):
- Request/response: `{"id":1,"method":"approvals.decide","params":{...}}` newline-delimited.
- Pub/sub: `{"sub":"events","filter":{"kinds":["pushback","approval"]}}` → server pushes `{"event":{...}}` frames.
- Handshake: `hello` exchanging `proto_version` (integer, start at 1); mismatch → shell shows "update RATO" panel.
- Tauri's Rust side holds one persistent socket client and re-exposes it to Svelte via Tauri commands/events.

---

## 4. Process/module breakdown

Cargo workspace `rato/`:

| Crate | Responsibility | Notes |
|---|---|---|
| `rat-proto` | IPC types, event/enum definitions, serde | shared by all binaries; semver discipline |
| `rat-core` | domain types, ids (ULIDs), clock abstraction, error types | no IO |
| `rat-policy` | risk classification, mode state machine (normal/private/away), retention rules, approval requirements | pure functions + sealed config; table-driven tests |
| `rat-store` | SQLite actor (single writer thread + bounded channel), migrations, FTS5, sqlite-vec, blob store for snapshots | exposes typed repos |
| `rat-sensors` | sensor hub: screen, mic, clipboard, shell-hook listener, procfs watcher, git watcher, transcript watcher, idle watcher; encrypted ring buffer | each sensor = supervised Tokio task with health state |
| `rat-derive` | OCR worker pool, transcription queue, classifiers (dev-relevance, language), embedder queue | bounded; drop-oldest backpressure for raw, never-drop for derived |
| `rat-memory` | memory service: observations, semantic notes, consolidation jobs, hybrid retrieval, disclosure ledger | `Retriever` trait isolates vec backend |
| `rat-brain` | critic/pushback engine, interruption governor, personality state machine, OpenAI Responses client, context-pack builder | core metric instrumentation lives here |
| `rat-workbench` | tmux server mgmt, worktree lifecycle, agent adapters (Claude Code/Codex/Aider/Gemini), merge-back flow | |
| `rat-inject` | terminal registry, detection scanner, injection executors (tmux/portal/ydotool/xdotool) | every injection requires an `ApprovalTicket` by type signature |
| `rat-voice` | PipeWire mic, wake word, VAD, whisper, Piper TTS, intent router | pre-wake ring is RAM-only by construction |
| `rat-daemon` (bin `ratd`) | composition root, RPC server, systemd notify (`sd_notify`), watchdog | |
| `rat-cli` (bin `rat`) | status, modes, approvals, `shell-init`, `install` (writes systemd units), doctor | |
| `apps/shell` (Tauri) | avatar window, dashboard window, popup dialogue windows | Svelte 5; `src-tauri` is a thin socket proxy |

Supervision: `ratd` runs a supervisor that restarts crashed sensor tasks with exponential backoff and surfaces persistent failures as a red sensor LED + Sensors-tab diagnostic. systemd `Restart=on-failure`, `WatchdogSec=30` with `sd_notify` heartbeats.

---

## 5. Sensor pipeline

```
sensor task ──► SensorFrame ──► Event Bus ─┬─► Ring Buffer Writer (raw, encrypted, 20 min)
                                           ├─► Deriver queues (OCR / STT / classify)
                                           └─► Signal detectors (stuck loop, ctx switch, error burst)
derived text ──► observations table ──► FTS5 + embed queue ──► vec index
```

**Sensors and cadence:**

| Sensor | Mechanism | Cadence | Raw → ring? | Derived output |
|---|---|---|---|---|
| Screen | portal ScreenCast → PipeWire; persistent restore token so consent is once | grab frame every 2 s; skip if perceptual hash (dHash) within distance 4 of last | yes (JPEG q70 segments) | OCR text blocks w/ window title via portal metadata; diffed against previous OCR to store only changes |
| Microphone | PipeWire 16 kHz mono | continuous | yes (Ogg/Opus 10 s segments) — **only post-consent**; pre-wake ring is RAM-only (§15) | transcripts (only when ambient-transcription toggle is ON; default OFF — only wake-word interactions are transcribed) |
| Clipboard | wlr data-control / arboard poll | on change | yes (encrypted text segments) | classified entries (code/stack-trace/url/secret-like); secret-like (regex: keys, tokens, passwords) are **never** derived or embedded, ring-only |
| Shell commands | shell hooks: preexec/precmd emit `{cmd, cwd, exit, duration, tty}` | per command | n/a (already structured) | `shell_cmd` observations |
| Terminal sessions | tmux control-mode client on `-L rato` + registered foreign sessions (§8) | streamed | n/a | pane output summaries for workbench panes |
| Process list | procfs scan | every 5 s, diff-based | n/a | process start/stop events for dev-relevant cmds (allowlist: compilers, test runners, agent CLIs, editors) |
| Git state | gix read of registered project roots + inotify on `.git/HEAD`, refs, index | on change, debounced 2 s | n/a | branch/HEAD/dirty-state/ahead-behind events; diffstat on commit |
| Project files | `notify` (inotify) on registered roots, gitignore-aware | on change, debounced | n/a | file-churn counters (no content stored from this sensor) |
| LLM CLI transcripts | watch registered transcript dirs (`~/.claude/projects/**.jsonl`, Codex/Aider/Gemini equivalents) | on change | n/a | parsed turns → `agent_output` observations |
| Idle | ext-idle-notify / XScreenSaver / logind | threshold 15 min | n/a | drives Away mode |

**Encrypted ring buffer:** directory `~/.local/state/rato/ring/{screen,audio,clipboard}/`; fixed 10-second segments, XChaCha20-Poly1305, **ephemeral per-run key** generated at daemon start and held only in `mlock`ed memory — a crash makes old ring data unreadable garbage, which is acceptable and a privacy feature. Ring writer deletes segments older than 20 min every 60 s. **Pinning** re-encrypts the segment with the persistent *pin key* (keyring-stored) and moves it to `~/.local/share/rato/pins/`; auto-pins get `expires_at = now+30d`, manual pins `expires_at = NULL`.

**Auto-pin triggers** (deriver-side classifier, cheap model `gpt-5-mini` on OCR/transcript deltas + local regex pre-filter): visible stack trace or panic, failing test wall, error dialog, copied error text, spoken design decision during wake interaction. Each auto-pin records `reason`.

**Signal detectors (local, no LLM):**
- *Stuck loop:* same normalized command failing (exit ≠ 0) ≥3 times in 15 min, or edit→test cycle on the same file ≥5 times in 20 min, or agent pane output with cosine-similar (>0.93) consecutive summaries.
- *Context switch:* focused-window project changes (from OCR window-title metadata + cwd events) more than 6×/30 min.
- *Error burst:* ≥10 error-classified lines/min in any watched pane.

**SensorGate:** one struct holding per-sensor `enabled/paused/private` flags; private mode flips screen/mic/clipboard to `paused` *and* sets `remote_personal_memory=deny` in the orchestrator. The avatar LEDs render directly from SensorGate's broadcast channel.

---

## 6. AI orchestration design

**Provider abstraction:** one trait in `rat-brain`, three backends, all thin `reqwest` wrappers (no SDK churn):

```rust
trait ChatBackend: Send + Sync {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, LlmError>;
    // ChatRequest: system, messages, json_schema (always set — structured outputs everywhere),
    //              route (Critic | Cheap), purpose, max_tokens
    fn provider(&self) -> Provider;        // OpenAi | Anthropic | OpenRouter
    fn model_for(&self, route: Route) -> &str;
}
```

| Backend | Endpoint | Auth | Structured outputs | Notes |
|---|---|---|---|---|
| `OpenAiResponsesBackend` (default) | `POST https://api.openai.com/v1/responses` | `Authorization: Bearer` | `response_format: json_schema` | `store: false` (no server-side retention), `metadata.purpose` |
| `AnthropicBackend` | `POST https://api.anthropic.com/v1/messages` | `x-api-key` + `anthropic-version: 2023-06-01` | `output_config: {format: {type: "json_schema", schema}}` | adaptive thinking left on (omit/`{type:"adaptive"}`); check `stop_reason == "refusal"` before parsing; never branch on `stop_details` |
| `OpenAiCompatBackend` (OpenRouter) | `POST {base_url}/chat/completions`, base_url default `https://openrouter.ai/api/v1` | `Authorization: Bearer` | `response_format: json_schema` | optional `HTTP-Referer`/`X-Title` attribution headers; same backend reusable for any OpenAI-compatible endpoint (local llama.cpp/Ollama later) |

Provider is chosen in `config.toml` (`[llm] provider = "openai" | "anthropic" | "openrouter"`), switchable in Settings; per-route model ids overridable per provider. No automatic cross-provider failover in v1 — a failing provider surfaces as a Sensors-tab `NET` warning. API keys live in Secret Service as `rato/openai`, `rato/anthropic`, `rato/openrouter`; `rat setup` prompts only for the chosen provider (plus OpenAI if embeddings are wanted). All calls logged to `api_calls` with provider, model, token counts and cost; every included memory/observation id recorded in `disclosures` (§9) regardless of provider.

**Model routing (deterministic defaults per provider):**

| Route | OpenAI (default) | Anthropic | OpenRouter |
|---|---|---|---|
| Critic slow-tick review, proposal generation, chat with operator | `gpt-5.1` | `claude-opus-4-8` | `openai/gpt-5.1` |
| Summarization (session/day), classification, auto-pin relevance, language tagging | `gpt-5-mini` | `claude-haiku-4-5` | `openai/gpt-5-mini` |
| Embeddings | `text-embedding-3-small` — **always OpenAI direct** (Anthropic and OpenRouter have no embeddings endpoint). Without an OpenAI key, embedding is disabled and retrieval runs FTS5-only with a Settings warning. | | |

**Critic/pushback loop:**

1. **Fast tick (30 s, local only):** evaluate signal detectors; raise candidate pushbacks from heuristics (stuck loop, away-drift, failing tests ignored, uncommitted >2h with large diff).
2. **Slow tick (5 min, LLM):** build a **context pack** — last 5 min of observation summaries, current git state, active task, top-8 retrieved memories (hybrid retrieval, §9), open proposals — and ask `gpt-5.1` for a structured verdict:
   ```json
   {"pushback": null | {
      "severity": "nudge|warn|block-suggest",
      "title": "...", "message_en": "...", "message_pt": "...",
      "evidence": [{"observation_id": "...", "quote": "..."}],
      "proposed_actions": [{"kind": "run_command|open_workbench_task|pin|note", "...": "..."}],
      "confidence": 0.0
   }}
   ```
   Pushbacks with no evidence ids are dropped (hard rule: critique must cite observations). Confidence <0.6 → logged, not shown.
3. **Event-triggered:** stuck-loop / error-burst / big-risky-diff signals trigger an immediate slow-tick.
4. **Interruption Governor:** token bucket per personality mode — Chaos Critic 1 popup/10 min (burst 2), Mentor 1/30 min, Quiet Analyst 1/2 h, all modes hard cap 8 popups/h. Suppressed pushbacks queue in the Pushback tab. Identical-evidence pushbacks dedupe for 24 h.
5. **Learning:** every shown pushback gets Useful / Dismiss / Snooze. Feedback updates (a) per-trigger-type threshold multipliers (simple multiplicative weights, persisted), (b) a behavioral memory note ("operator dismisses lint-related pushback during prototyping"). **Core metric:** acceptance rate and time-to-decision per trigger type, charted in Metrics.

**Prompt-injection hardening:** all observed text enters prompts inside fenced blocks labeled `UNTRUSTED OBSERVATION`; the system prompt states instructions inside observations must be treated as data; proposals are validated against the JSON schema and then against the PolicyEngine — the model cannot mint approvals, only proposals.

**Agent adapters** (trait): `name()`, `detect_binary()`, `headless_cmd(task, worktree)`, `interactive_cmd()`, `parse_transcript(path)`, `transcript_dirs()`, `health()`. Implementations for Claude Code, Codex CLI, Aider, Gemini CLI; new tools are one file each.

---

## 7. tmux/worktree workbench design

**Server:** dedicated `tmux -L rato` (socket isolated from user's own tmux). `ratd` keeps one control-mode client (`tmux -C -L rato attach`) for the event stream (`%output`, `%window-add`, `%session-changed`) and shells out for commands.

**Layout convention:** session per project (`rato-<project-slug>`); window per task (`t<task-id>-<slug>`); pane 0 = agent CLI, pane 1 (optional) = test/watch pane. Operator can attach anytime: `tmux -L rato attach` (read/write — it's their machine; the avatar shows "operator attached" while they're in).

**Worktrees:** all agent work happens at `~/.local/share/rato/worktrees/<repo-hash>/<task-id>/`, branch `rato/<task-slug>` cut from the operator's current HEAD (or a chosen base). Lifecycle:

1. `git worktree add` (approval **not** required — it doesn't touch the live repo's working tree).
2. Agent runs in the worktree (headless or interactive pane). Commits stay on `rato/*` branches.
3. **Review:** daemon produces diffstat + full diff vs base; dashboard Workbench tab renders it.
4. **Merge-back is always R2** (§11): proposal shows target branch, diff, test results from the worktree. On approval, the executor runs in the *operator repo*: `git merge --no-ff rato/<slug>` if fast-mergeable, else offers `git cherry-pick` or "leave branch for manual merge". Never auto-resolve conflicts.
5. Cleanup: `git worktree remove` + branch deletion after merge or 14 days of inactivity (with a dashboard notice; branch is preserved as a bundle blob for 30 days).

**Guards:** workbench commands run with `cwd` pinned inside the worktree; env scrubbed (no `SSH_AUTH_SOCK` pass-through unless task config opts in); `GIT_DIR` indirection checked so an agent can't `git -C` its way into the live repo — the policy engine flags any proposed command whose resolved paths escape the worktree as R2+.

---

## 8. Terminal detection and injection design

**Detection (read-only, every 10 s):** scan `/proc/*/cmdline` for adapter binary names (`claude`, `codex`, `aider`, `gemini`, configured extras). For each hit: resolve controlling TTY via `/proc/<pid>/fd` → `/dev/pts/N`; map to tmux pane if any tmux server's `list-panes -a -F '#{pane_tty} #{session_name}:#{window_index}.#{pane_index}'` matches; identify terminal emulator by walking the parent chain. Distinguish: **rato workbench** (our `-L rato` socket → auto-registered), **foreign**.

**First sighting of a foreign LLM terminal** → avatar dialogue: *"I see Claude Code running in kitty (pts/4), project `walljobs-api`. Is this your working terminal?"* Choices: **Operator terminal** (observe transcripts only; injection always requires per-event approval), **Make it a workbench** (register for managed use), **Ignore** (remembered per tty+cmd hash). Stored in `terminals` with role.

**Injection paths, in strict preference order per environment:**

| # | Path | Env | Mechanism |
|---|---|---|---|
| 1 | tmux | any pane we can address | `tmux load-buffer -` + `paste-buffer -p -t <target>` (bracketed paste), then `send-keys -t <target> Enter` if approved as paste-and-enter |
| 2 | XDG portal RemoteDesktop (libei) | Wayland w/ portal support | sanctioned OS path; persistent session token requested once |
| 3 | ydotool | Wayland w/o working portal | requires uinput setup; `rat doctor` guides it |
| 4 | xdotool (XTEST) | X11 | activate window, verify `_NET_ACTIVE_WINDOW`, then type/paste |

**Injection ceremony (always, no exceptions):**
1. Approval record (R2) shows: exact bytes to paste (rendered verbatim, monospace), target (`session:window.pane` or window title + tty), whether Enter is included, expiry (default 10 min).
2. Just-in-time recheck at execution: pane/window still exists, `pane_current_command` matches what the approval recorded, and — desktop paths — focused window equals target. On Wayland, where focus can't be verified, a 3-second on-screen countdown overlay with Cancel runs first and injection aborts on any user keyboard/mouse activity during the countdown.
3. Result (success/abort/changed-target) appended to the approval record.
4. Away mode (§11) hard-blocks all injection regardless of standing approvals.

Bracketed paste is always used where supported so multi-line payloads can't auto-execute line-by-line; Enter is a separate, explicitly-approved keystroke.

---

## 9. Memory and retrieval strategy

**Four layers, all in SQLite:**

| Layer | Table | Content | Lifetime |
|---|---|---|---|
| Raw | ring + `pins` | encrypted screen/audio/clipboard segments | 20 min / 30 d / manual |
| Episodic | `events`, `observations` | structured events; OCR/transcript/clipboard/shell/agent text | long-term (pruned, §12) |
| Semantic | `memories` | LLM-written typed notes: `project`, `personal` (behavioral/performance, cross-project), `preference`, `episode_summary` | long-term |
| Index | FTS5 + `vec0` tables | lexical + 1536-d embeddings over observations and memories | follows source row |

**Consolidation jobs:**
- *Sessionizer (continuous):* groups events into `work_sessions` (project, start/end, gap threshold 25 min) — feeds the Calendar.
- *Hourly:* embed any unembedded observations; summarize each closed work session (`gpt-5-mini`) with citations (event ids).
- *Nightly (03:30):* day summary; update/merge semantic notes (create/strengthen/contradict — contradicted notes get confidence decay, archived below 0.2); recompute metrics; prune (§12).

**Retrieval (hybrid, deterministic):** given a query — FTS5 BM25 top-40 + vec cosine top-40 → Reciprocal Rank Fusion (k=60) → filters (project scope, type) → recency boost (`score × (1 + 0.25·e^(−age_days/14))`) → top-N. Personal memories are included cross-project; project memories only for the active project unless explicitly requested.

**Disclosure ledger:** every Responses API call writes a `disclosures` row listing exactly which `memory_ids`/`observation_ids` were serialized into the prompt, plus purpose and model. Memory tab renders per-note "sent to remote AI N times, last on …". Private mode → context packs exclude `personal` memories and the disclosure writer enforces it (assert + drop).

---

## 10. Data model/schema

SQLite, WAL, all ids ULID strings, all timestamps integer ms UTC. Canonical DDL (abridged but structurally complete):

```sql
events(id PK, ts, kind, source, project_id NULL, session_id NULL, payload JSON, lang NULL);
projects(id PK, root_path UNIQUE, name, vcs, first_seen, last_seen, settings JSON);
work_sessions(id PK, project_id, started, ended NULL, kind, summary TEXT NULL,
              ctx_switches INT, commands INT, tests_run INT, tests_failed INT);
observations(id PK, event_id, ts, kind /*ocr|transcript|clipboard|shell_cmd|git|agent_output|note*/,
             project_id NULL, content TEXT, meta JSON, embedded INT DEFAULT 0);
observations_fts(content, tokenize='unicode61');            -- contentless, synced by triggers
vec_observations(embedding float[1536], obs_id TEXT);        -- vec0 virtual table
memories(id PK, type /*personal|project|preference|episode_summary*/, project_id NULL,
         title, body, confidence REAL, created, updated, source_event_ids JSON, archived INT);
memories_fts(...); vec_memories(embedding float[1536], memory_id TEXT);
pins(id PK, kind /*auto|manual*/, media /*screen|audio|clipboard*/, path, created,
     expires_at NULL, reason, meta JSON);
approvals(id PK, created, kind /*command|inject|merge_back|global_install|shell_startup|
          mcp_install|live_repo|dotfile_edit_escalated/*, risk INT, title, reason,
          cwd NULL, target NULL /*tmux target | window+tty*/, agent_identity,
          payload JSON /*exact command/bytes/diff*/, expected_impact JSON,
          expires_at, status /*pending|approved|denied|expired|cancelled*/,
          decided_at NULL, decided_via NULL /*popup|dashboard|voice|cli*/, decision_note NULL,
          execution JSON NULL /*started, ended, exit_code, output_ref, verified_target*/);
actions(id PK, approval_id NULL, kind, payload JSON, started, ended NULL, exit_code NULL, output_blob NULL);
agent_runs(id PK, adapter, task_title, project_id, worktree_path, branch, tmux_target NULL,
           mode /*headless|interactive*/, status, tokens JSON, cost_usd REAL, started, ended NULL,
           result_summary NULL, diffstat JSON NULL);
terminals(id PK, tty, pid, emulator, tmux_target NULL, role /*operator|workbench|foreign|ignored*/,
          detected_cmd, project_guess NULL, first_seen, last_seen, confirmed INT);
pushbacks(id PK, ts, mode, trigger, severity, title, message_en, message_pt, evidence JSON,
          proposals JSON, confidence REAL, status /*queued|shown|accepted|dismissed|snoozed|expired*/,
          decided_at NULL, latency_ms NULL);
disclosures(id PK, ts, api_call_id, model, purpose, memory_ids JSON, observation_ids JSON);
api_calls(id PK, ts, model, purpose, tokens_in, tokens_out, cost_usd, ok INT, error NULL);
dotfile_edits(id PK, ts, path, before_blob, after_blob, validator, validation_ok INT,
              reason, source /*auto|approved*/, reverted_by NULL);
blobs(id PK, sha256 UNIQUE, bytes BLOB, created);            -- snapshots, outputs, bundles
settings(key PK, value JSON, updated, updated_via);
policy_audit(id PK, ts, key, old JSON, new JSON, via);
metrics_daily(date, project_id NULL, metrics JSON, PRIMARY KEY(date, project_id));
voice_utterances(id PK, ts, lang, text, intent NULL, wake_word, handled INT);
schema_migrations(version PK, applied_at);
```

Indexes on every `(ts)`, `(project_id, ts)`, `approvals(status, expires_at)`, `terminals(tty)`. Blob store keeps dotfile snapshots, command outputs (>32 KB outputs go to blobs, referenced by id), and worktree bundles. Single DB file `~/.local/share/rato/rato.db`; nightly `VACUUM INTO` backup kept ×7.

---

## 11. Approval and permission model

**Risk tiers (PolicyEngine, table-driven, not adaptive):**

| Tier | Meaning | Examples | Gate |
|---|---|---|---|
| R0 | Read-only observation | sensors, procfs, git reads, `.claude`/`.agents` indexing, transcript parsing | autonomous |
| R1 | Reversible managed writes | `.claude`/`.agents` edits (snapshot+diff+validate+log+revert), known MCP config edits referencing already-installed binaries, worktree creation, workbench-internal file writes, pins | autonomous **with audit row + dashboard feed entry** |
| R2 | Operator-visible side effects | command execution outside worktrees, terminal injection, merge-back to live repo, any live-repo write, project-local installs, MCP config pointing at new binaries | approval required (popup + dashboard), default expiry 10 min (inject) / 60 min (commands) |
| R3 | System-level / hard to reverse | global installs, downloading MCP servers or dev tools, shell startup file changes, anything touching `~/.config` outside known-safe list, `git push --force`, package manager with sudo (sudo itself: refused, never proposed) | approval + **typed confirmation** of a short slug; never approvable by voice |

**Approval record** always carries: cwd, reason, risk, target terminal/session, expected file impact, agent identity (which adapter/model proposed it), expiry, and after execution the result — matching the `approvals` schema above. **R3 install approvals** additionally render: source URL, version, exact command, permissions implications, rollback plan, and config changes (structured fields in `payload`/`expected_impact`).

**Scopes:** decisions can be "this once" (default), "this session", "this project, this command pattern" (stored as policy exceptions, visible and revocable in Settings → Permissions; pattern = exact argv prefix match, no regex). Tier *upgrades* by config are allowed (make R1 require approval); *downgrades* below the table are allowed only per the spec's "unless explicitly configured otherwise" — surfaced with a persistent warning chip in Settings, and R3 can never be downgraded.

**Modes:**
- **Private mode** (manual toggle, voice-toggleable): pauses screen/mic/clipboard capture and remote personal-memory sharing; avatar visibly "blindfolded".
- **Away mode** (auto after 15 min idle; exits on input): blocks terminal injection and global installs *even if approved* — affected approvals park as `pending` with reason "away"; avatar sleeps. Read-only observation continues per policy.

**Hard invariant:** adaptive/personality systems cannot alter tiers, retention, network policy, or approval requirements (crate-boundary enforced + `policy_audit` records the only legal mutation path: Settings UI/CLI).

---

## 12. Retention/pruning policy

| Data | Retention | Mechanism |
|---|---|---|
| Raw ring (screen/audio/clipboard) | 20 min | ring writer deletes; ephemeral key makes leaks moot |
| Pre-wake audio | seconds (RAM ring) | never written, never transcribed |
| Auto-pinned raw | 30 d hard-delete | nightly pruner; deletion logged |
| Manual pins | until user deletes | — |
| Observations (OCR/transcripts/clipboard text) | 180 d full → then only rows cited by memories/pins/summaries survive; rest deleted with their FTS/vec rows | nightly pruner, citation-aware |
| Work sessions, summaries, metrics, command history | indefinite | compact by design |
| Semantic memories | indefinite; confidence decay archives contradicted notes | nightly consolidation |
| Approvals, actions, disclosures, dotfile_edits, policy_audit | indefinite (audit) | exempt from pruning |
| API call logs | 365 d | nightly |
| Worktree branch bundles | 30 d post-cleanup | nightly |
| DB backups (`VACUUM INTO`) | 7 rotating | nightly |

Pruner runs in the 03:30 job, executes `DELETE` batches ≤5k rows to keep WAL small, logs counts to an event, and renders "last prune" in the Sensors tab. "Delete everything about X" (project or time range) is a Settings action that cascades across observations, memories, pins, vec/FTS, and is itself audit-logged.

---

## 13. Avatar and PS2 HUD design system

**Avatar window:** Tauri transparent, undecorated, `always_on_top`, skip-taskbar, non-resizable, 280×280, default bottom-left with 16 px margins, position persisted. Cursor pass-through by default; a Three.js raycast hit-test against the rat mesh toggles `set_ignore_cursor_events(false)` so only the rat is clickable. Drag to reposition (drag threshold 6 px so click ≠ drag). Click = quick-actions radial (pause sensors, private mode, new workbench task, pin last 2 min, open pushback queue). Double-click = dashboard. Right-click = mode/personality menu. On Wayland w/o always-on-top support, fall back to a regular window and document per-compositor rules (`rat doctor` prints them).

**Model spec (Blender → glTF):** ~2,200 tris, single 256×256 nearest-filtered texture, 22-bone skeleton, clips: `idle`, `idle_groom`, `alert`, `point`, `judge` (arms crossed), `typing`, `talk`, `sleep` (away), `blindfold_on` (private). PS2 look: vertex-precision snap shader (quantize clip-space verts to a 240-line grid), affine-ish texture wobble (perturb UV interpolation), no mipmaps, flat ambient + one directional light, 4-bit ordered-dither post pass rendered at 320×320 then upscaled nearest.

**Sensor LEDs:** a compact status strip above the rat — `SCR` `MIC` `CLP` `NET` chips, green = active, amber = paused, red ring = private; driven directly from SensorGate. This strip cannot be hidden (no-covert invariant).

**Personality modes** (auto-switch from behavior signals, right-click override; override sticks until cleared): **Mentor** (default), **Chaos Critic** (frequent, snarky, governor-limited per §6), **Quiet Analyst** (flow detected → minimal), **Hype** (post-milestone), **Rubber Duck** (operator talking through a problem). Mode changes alter interruption budget, message tone, outfit/posture (per-mode accessory meshes + idle clip), and Piper voice parameters (length-scale/pitch) — and **nothing else** (§11 invariant). Psychology-adjacent observations require ≥3 evidence citations spanning ≥2 days and must be phrased as behavioral observations ("you've restarted this refactor 4 times this week"), enforced by the pushback JSON schema's `evidence` minimum for `behavioral` trigger types.

**HUD-PS2 design system (CSS, no Tailwind/shadcn):** design tokens in `tokens.css`:
- Palette: bg `#0B0E14`, panel `#141A24`, panel-raised `#1C2433`, line `#3A4860`, text `#C8D4E0`, accent `#7CFF6B` (acid green), warn `#FFB02E`, danger `#FF5C5C`, info `#5CC8FF`.
- Borders: 2 px solid, square corners, top/left 1 px lighter + bottom/right 1 px darker bevel; panel headers as chunky title bars with notched corners (CSS `clip-path`).
- Textures: 2 tiling PNGs (`dither8.png` 4-bit Bayer overlay at 6 % opacity, `scanline.png` on the avatar bubble only).
- Type: **Departure Mono** for headers/labels (pixel), **IBM Plex Mono** for body/code; sizes 11/13/15/20 px, no font smoothing on Departure.
- Components (Svelte, in `ui/hud/`): `HudPanel`, `TitleBar`, `MeterBar` (segmented game meter), `StatusChip`, `MissionLog` (calendar rows), `DialogueBox` (avatar speech: typewriter text, corner tail, OK/Dismiss chunky buttons), `ApprovalCard` (risk-striped border: green/amber/red/red-double), `TabBar`, `DataTable`, `Sparkline` (canvas, 1 px steps), `Toggle` (slide switch w/ click sound). Motion: 120 ms steps (no easing curves — stepped keyframes for the retro feel). All components localized via Fluent.

---

## 14. Dashboard information architecture

Single Tauri window, left `TabBar`, content panels. **Default route: Now.**

| Tab | Content |
|---|---|
| **Now / Active Work** | current project, current work session meter (duration, ctx switches), live signal chips (stuck loop? tests red?), last 3 pushbacks, running agent tasks w/ MeterBars, today's API cost; quick actions |
| **Calendar** | auto-generated mission-log timeline (day/week zoom): work-session blocks per project; in-block glyphs for commands, test runs (pass/fail color), agent usage, approvals, pins, stuck loops, context switches; row click → session detail w/ summary + events; week footer shows performance trends. Not a task manager — no creation UI |
| **Pushback** | queue + history; filters by trigger/status; Useful/Dismiss/Snooze; acceptance-rate sparkline per trigger type |
| **Workbench** | tmux sessions/windows live (read-only pane tail via control client), agent runs table, worktree list with diffstat, merge-back review screen (diff viewer, test results, Approve/Deny) |
| **Memory** | semantic notes browser (type/project filters, confidence, citations), search (hybrid retrieval), pins gallery w/ expiry badges, disclosure ledger per note |
| **Metrics** | game-style meters & sparklines: focus time, ctx switches/h, test pass rate, stuck loops/day, pushback acceptance & time-to-decision, agent spend, command volume |
| **Sensors** | SensorGate board (per-sensor state, health, last frame), ring-buffer occupancy meter, prune log, "pin last N minutes" |
| **Approvals** | pending queue (same ApprovalCards as popups), full audit history with execution results, standing permission exceptions list (revoke buttons), dotfile/MCP edit feed with diff viewer + one-click revert |
| **Settings** | modes, language, personality override, retention dials (within policy floor), permissions, API key status (keyring), models, voice/wake toggles, `rat doctor` output |

Popups (approval & pushback) are tiny frameless Tauri windows using `DialogueBox`/`ApprovalCard`, bottom-left above the avatar, max 2 stacked; overflow collapses into a "+N queued" chip.

---

## 15. Bilingual voice/wake-word architecture

```
PipeWire mic 16 kHz ─► RAM ring (8 s, mlocked, never persisted, never transcribed)
        │
        ├─► openWakeWord (4 ONNX models: "rat", "hey rat", "rato", "ei rato")
        │        └─ on wake: chime + avatar ear-perk + MIC chip pulses
        ▼
   Silero VAD endpointing (max 30 s utterance)
        ▼
   whisper-rs (small | large-v3-turbo on GPU), language auto-detect constrained to {en, pt}
        ▼
   Intent router
     ├─ local grammar (regex/keyword, per language): pause/resume sensors, private mode on/off,
     │   open dashboard, "pin that"/"pina isso" (pins last 2 min), snooze, mode switch,
     │   approval decisions (see gate below)
     └─ fallback: operator chat → orchestrator (gpt-5.1) → reply text → Piper TTS
              (voice: en_US-lessac-medium / pt_BR-faber-medium) + DialogueBox
```

- **Pre-wake audio:** the 8 s ring exists only to give whisper leading context *after* a wake; pre-wake content is RAM-only, overwritten continuously, and is never written, transcribed, or embedded. The continuous-ambient ring segment writer (§5) is a separate path that only runs when the operator has explicitly enabled ambient audio capture (default ON for ring-buffer-only retention, OFF for ambient transcription).
- **Language policy:** `voice_utterances.lang` from whisper's detection; typed chat language via `lingua-rs` (en/pt only). The most recent operator utterance/message sets `last_language`; avatar speech, popups, and TTS use it (`message_en`/`message_pt` are always both generated; render-side picks).
- **Voice approvals gate:** voice may decide an approval only if (a) the approval popup is currently visible, (b) risk ≤ R2, and (c) the utterance includes the approval's two-word slug shown on the card (e.g. "approve amber-fox" / "aprovar amber-fox"). R3 is never voice-approvable. Decision recorded with `decided_via='voice'` plus the utterance id.
- Wake-word models: train openWakeWord custom models offline using Piper-generated synthetic positives (both accents) + noise negatives; ship the 4 `.onnx` files as assets; per-model threshold tuned to ≤1 false accept/8 h (measured in soak test).

---

## 16. MCP / `.claude` / `.agents` integration

**Known config surface (read autonomously, R0):** `~/.claude.json`, `~/.claude/settings.json`, project `.claude/**`, project `.mcp.json`, `.agents/**`, Codex `~/.codex/config.toml`, Gemini `~/.gemini/settings.json`, Aider `.aider.conf.yml`. Indexed into observations (kind `note`, meta `config`) so the critic can reference agent setup.

**DotfileEditor service (single chokepoint for every write):**
1. Read current content → store `before_blob` (sha-addressed).
2. Compute edit → validate: must parse (JSON/JSONC/TOML/YAML per file), schema-check known keys, and for MCP entries verify the referenced command exists on `$PATH` or as an absolute file.
3. Apply atomically (write temp + rename) → store `after_blob`, diff, reason, source.
4. Emit event → Approvals-tab "Config Changes" feed entry with diff viewer and **one-click revert** (revert = new DotfileEdit writing `before_blob`, linked via `reverted_by`).

**Policy mapping:** `.claude`/`.agents` edits and MCP config edits that only reference already-installed local binaries = **R1** (automatic, audited, reversible). Adding an MCP server that requires download/install, or any global tool install = **R3** with the full install card (source, version, exact command, permissions, rollback plan, config changes). Shell startup file changes (`.bashrc`/`.zshrc`/`fish` config — including the one-line `rat shell-init` hook) = **R3** always; the installer asks once and shows the exact line.

Validation failure → edit aborted and surfaced as a warning, never half-applied.

---

## 17. Security/privacy threat model

**Assets:** raw screen/audio/clipboard (highest sensitivity), derived memory (behavioral profile), OpenAI API key, approval integrity (ability to make RATO act), operator's repos and shell.

| Threat | Vector | Mitigation |
|---|---|---|
| Other local users read data | files, socket | dirs `0700`, socket `0600` in `$XDG_RUNTIME_DIR/rato`, DB `0600`; ring encrypted w/ ephemeral key; pins encrypted w/ keyring key |
| Prompt injection via observed content | OCR'd web page / agent output says "run rm -rf" | observations wrapped as UNTRUSTED data; model can only emit schema-validated *proposals*; PolicyEngine classifies independently of model claims; approval cards render from structured fields only (no model-authored markup), so remote content can't forge ceremony |
| Malicious/compromised MCP server or agent CLI | adapter output, tool install | installs are R3 with provenance card; adapter output treated as untrusted; workbench env scrubbed; worktree path-escape detection (§7) |
| Injection hits wrong target | pane/window changed | JIT target recheck, focus verify (X11), countdown+activity-abort (Wayland), bracketed paste, Enter approved separately |
| Secrets leak into memory/remote | clipboard passwords, keys on screen | secret-pattern classifier blocks derivation/embedding of secret-like clipboard; OCR redaction pass for common key formats before storage; disclosure ledger for everything that does go remote; private mode |
| Daemon compromise via API responses | malformed JSON, oversized | strict serde, size caps, no dynamic code paths |
| Approval fatigue → rubber-stamping | too many R2s | scoped standing exceptions (visible/revocable), interruption governor, batching in dashboard |
| Physical absence abuse | someone uses the desk | Away mode blocks injection/global installs; R3 typed confirmation |
| Network exfil beyond intent | — | egress only to the configured LLM provider (`api.openai.com`, `api.anthropic.com`, `openrouter.ai`) + adapters' own endpoints (adapters run as the user; documented, not proxied — out of scope to sandbox trusted dev tools); `store:false` on OpenAI calls; disclosure ledger covers all providers |
| OS security bypass | screen/input grabbing | portals only (ScreenCast, RemoteDesktop); no `/dev/uinput` unless user opts into ydotool; never sudo |

Out of scope: malware already running as the user (it owns everything anyway), multi-user adversarial setups (product is single-trusted-user by definition).

---

## 18. MVP milestones

| M | Deliverable | Acceptance criteria |
|---|---|---|
| **M0 — Spine** (wk 1–2) | workspace, `ratd` + `rat`, UDS RPC, SQLite + migrations, events table, systemd units, `rat install`/`doctor`, shell alias | `rat status` round-trips; daemon survives reboot via systemd --user; events persisted |
| **M1 — Cheap sensors** (wk 3–4) | shell hooks, procfs, git watcher, clipboard, idle/Away, project registry, sessionizer v1 | commands/git/clipboard land as observations; Away triggers at 15 min; sessions form correctly on synthetic timelines |
| **M2 — Shell** (wk 5–6) | Tauri avatar (static idle anim, LEDs, drag, menus) + dashboard skeleton (Now, Sensors, Settings) + HUD-PS2 tokens/components | avatar always-on-top on X11 + GNOME/KDE Wayland; LEDs track SensorGate live; popups render |
| **M3 — Memory + Critic v1** (wk 7–9) | embeddings, FTS, hybrid retrieval, consolidation jobs, OpenAI client, slow-tick critic, pushback popups + tab, governor, feedback loop, bilingual messages | end-to-end: induced stuck loop (scripted failing test ×3) yields a cited pushback within 5 min, rate-limited, dismissible; disclosures recorded |
| **M4 — Workbench** (wk 10–12) | tmux `-L rato` mgmt, worktrees, Claude Code + Codex adapters (headless+interactive), approval service + ApprovalCards, merge-back flow, Workbench tab | agent completes a scripted task in a worktree; merge-back requires approval and lands clean; denial leaves live repo untouched (asserted by test) |
| **M5 — Eyes** (wk 13–15) | portal ScreenCast, ring buffer, OCR pipeline, auto/manual pins, retention pruner, Calendar tab | 24 h soak: CPU <8 % avg, ring bounded at 20 min, OCR observations searchable, pins expire correctly (clock-skewed test) |
| **M6 — Voice** (wk 16–17) | wake word ×4, VAD, whisper, intents, Piper TTS, voice-approval gate | both languages wake & command reliably; pre-wake audio provably never persisted (code audit + fs watch test); slug-gated voice approval works |
| **M7 — Polish** (wk 18–19) | full PS2 avatar model + shaders + personality modes, Metrics/Memory/Pushback tabs complete, terminal detection/injection chain, MCP/DotfileEditor | foreign Claude terminal detected & classified; approved paste-and-enter executes with ceremony; `.claude` edit auto-applies with revert |
| **M8 — Hardening** (wk 20) | threat-model checklist pass, Aider+Gemini adapters, soak tests, docs, backup/restore | 7-day soak no leaks (RSS flat ±10 %), all §19 suites green |

---

## 19. Testing strategy

- **Unit (cargo-nextest):** PolicyEngine table-driven (every kind × tier × mode × exception → expected gate); sessionizer golden tests (synthetic event streams → `insta` snapshots); retention pruner with `proptest` (invariants: never delete cited observations, audit tables, manual pins; always delete expired auto-pins); retrieval fusion determinism; language detection; ring-buffer rotation (fake clock — all time via the `rat-core` clock abstraction).
- **Integration — tmux/worktrees:** harness spins `tmux -L rato-test` + throwaway git repos; **fake agent CLI** (`fakeagent` test binary: scriptable outputs/exit codes/transcripts) registered as an adapter; scenarios: task→worktree→commit→merge-approval→merge; denial leaves operator repo byte-identical (hash tree before/after); pane-gone-before-injection aborts; control-mode event parsing.
- **Integration — store:** migrations up from every prior version; crash-recovery (kill -9 mid-write, reopen, WAL replay); FTS/vec trigger sync.
- **Injection safety tests:** approval-required-by-construction (type-level: compile-fail test that `Injector::execute` is unreachable without `ApprovalTicket`); JIT recheck failures abort and record.
- **Sensor tests:** recorded PipeWire fixtures piped through OCR/STT derivers → snapshot outputs; secret-classifier corpus (true/false positives tracked).
- **Voice:** WAV fixture suite per wake word per language (accents, noise) with accept/reject thresholds asserted; pre-wake non-persistence test (inotify watch on all writable dirs during pre-wake audio).
- **UI (Playwright against `vite dev` + mocked daemon socket):** tab navigation, ApprovalCard render-from-structured-fields (injection of HTML in payload renders inert), DialogueBox typewriter, locale switch, LED state binding.
- **E2E smoke (nightly CI, headless Wayland via `cage` or Xvfb):** boot daemon+shell, scripted day (commands, failing tests, agent task), assert: pushback fired, calendar populated, approvals audited.
- **Soak:** 7-day run with synthetic activity generator; assert RSS/file-handle/ring-size bounds, wake-word false-accept rate, DB growth curve.
- **Security checks in CI:** `cargo deny` (licenses/advisories), socket/db permission asserts, grep-gate forbidding `Command::new("sudo")`.

---

## 20. Final "build this" bill of materials

**Repository `rato/` (monorepo):**
```
crates/ rat-proto rat-core rat-policy rat-store rat-sensors rat-derive
        rat-memory rat-brain rat-workbench rat-inject rat-voice rat-daemon rat-cli
apps/shell/            # Tauri v2 + Svelte 5 (src-tauri proxy, ui/hud components, avatar/, dashboard/)
assets/avatar/rat.glb  # + textures, dither8.png, scanline.png
assets/wakewords/{rat,hey_rat,rato,ei_rato}.onnx
assets/voices/{en_US-lessac-medium,pt_BR-faber-medium}.onnx
assets/fonts/{DepartureMono,IBMPlexMono}/
i18n/{en-US,pt-BR}/*.ftl
packaging/systemd/{ratd.service,rato-shell.service}
packaging/shell/{rat.bash,rat.zsh,rat.fish}
justfile  docs/  tests/{harness,fixtures}
```

**Key Rust crates:** tokio, clap, tracing(+subscriber+journald), serde/serde_json, rusqlite(+rusqlite_migration), sqlite-vec, reqwest, ulid, ashpd, pipewire, leptess, image/imageproc, ort, whisper-rs, wl-clipboard-rs, arboard, procfs, sysinfo, gix, notify, portable-pty, keyring, chacha20poly1305, notify-rust, fluent, lingua, insta, proptest.

**Frontend deps:** @tauri-apps/api v2, svelte 5, vite, three, @fluent/bundle, playwright (dev).

**External binaries (checked by `rat doctor`):** tmux ≥3.3, git ≥2.40, tesseract 5 + `eng`/`por` traineddata, piper, ydotool (optional), xdotool (X11), wl-clipboard.

**Models/assets to produce:** whisper ggml `small` (auto-download) + optional `large-v3-turbo` Q5; 4 openWakeWord models (trained offline w/ Piper synthetic data); rat glTF (~2,200 tris, 256² texture, 22 bones, 9 clips — commissioned or built in Blender from a blockout: body, head, tail, ears, accessory anchor points for personality outfits).

**Secrets/config:** LLM API keys in Secret Service — `rato/openai`, `rato/anthropic`, `rato/openrouter` (`rat setup` prompts for the chosen provider; OpenAI key additionally enables embeddings); `~/.config/rato/config.toml` for non-secret settings (provider, models, thresholds, adapters, registered projects).

**Services:** `ratd.service` (default.target, watchdog 30 s), `rato-shell.service` (graphical-session.target). Shell alias from `rat shell-init` (R3-gated write or manual paste).

**Estimated footprint:** idle CPU <3 % (no GPU), ~350 MB RSS daemon + ~250 MB shell; DB growth ≈ 5–15 MB/day before pruning. Estimated effort: ~20 engineer-weeks to M8 per §18.

— End of specification v1.0 —
