# RATO functionality test checklist

Manual verification checklist per subsystem. Automated coverage: `cargo test --workspace`
(all crates) + `cd apps/shell && npm run check`. Items marked ⚙ are scriptable; 👁 need eyes.

## M0 — Spine (daemon + CLI + store)

- [ ] ⚙ `systemctl --user status ratd` → active (running); survives reboot (enabled)
- [ ] ⚙ `rat status` → daemon version + event count round-trips over the UDS socket
- [ ] ⚙ `rat emit '{"kind":"note","payload":{"t":"hello"}}'` then `rat events` → event persisted
- [ ] ⚙ socket and DB are 0600/0700 (`ls -la /run/user/$UID/rato ~/.local/share/rato`)
- [ ] ⚙ kill -9 the daemon mid-write → restart → `rat status` works (WAL recovery)

## M1 — Sensors

- [ ] ⚙ run a command in a hooked shell → `rat observations --kind shell_cmd` shows it with exit code
- [ ] ⚙ `git commit` in a watched repo → `rat observations --kind git` shows HEAD move
- [ ] ⚙ copy text → `rat observations --kind clipboard_text` shows it (secrets redacted: copy an
      AWS-style key, confirm `[REDACTED]`)
- [ ] ⚙ `rat projects` lists projects auto-registered from cwd of observed activity
- [ ] 👁 idle 15 min (or set idle threshold low) → `rat mode` flips to away; input flips back
- [ ] ⚙ sessions form: `rat sessions` groups activity with 25-min gap rule

## M2 — Shell (avatar + dashboard)

- [ ] 👁 avatar visible bottom-left, always-on-top, front-facing biped rat bust with hands
- [ ] 👁 idle animation: breathing, sway, blink; occasional one-hand wave; away mode slumps + zzz
- [ ] 👁 LEDs: NET green with daemon up; `systemctl --user stop ratd` → NET red ≤ 2 s; start → green
- [ ] 👁 drag the tape grip → window moves; relaunch shell → reopens at the dragged spot;
      `rm ~/.local/share/rato/avatar-pos.json` + relaunch → flush bottom-left default
- [ ] 👁 double-click rat → dashboard opens with THUG2 paper/sticker styling; close hides (avatar lives)
- [ ] 👁 Now tab shows real sessions/observations; Sensors tab shows live sensor states

## M3 — Memory + Critic

- [ ] ⚙ `rat setup --keys-dir ~/rato/keys` → 3 keys stored; `rat doctor` shows all present;
      keys survive into a NEW process (not the in-memory mock)
- [ ] ⚙ `rat llm-status` → provider correct, critic_enabled true when key present
- [ ] ⚙ induce stuck loop: same failing cmd ×3 within 10 min
      (`rat emit-shell --cmd "pytest tests/x.py" --cwd <project> --exit 1` ×3)
      → `rat pushbacks` shows a `shown` pushback **within 5 min**
- [ ] ⚙ pushback cites ≥1 evidence observation id that exists in the DB (no fabricated ids)
- [ ] ⚙ `disclosures` row written for the critic call (model, purpose, observation ids);
      `api_calls` row has token counts
- [ ] ⚙ repeat the loop immediately → second pushback is `queued` (governor) and identical
      evidence within 24 h is deduped entirely
- [ ] ⚙ `rat pushbacks feedback <id> dismiss` → status dismissed, latency recorded
- [ ] 👁 dashboard Pushback tab lists it with severity chip; Useful/Dismiss/Snooze work
- [ ] 👁 avatar speech bubble appears for a new `shown` pushback; ✓/✕ buttons work; auto-hides 30 s
- [ ] ⚙ `rat search "<term>"` returns ranked hits; with embeddings blocked (403) search still
      works FTS-only and `llm-status` reports embedding degradation
- [ ] ⚙ hourly job embeds new observations (when embeddings available) and summarizes closed
      sessions into `episode_summary` memories with citations

## M4 — Workbench

Verified live 2026-06-11 against an isolated daemon (sandbox `XDG_*` dirs) + scratch repo
`~/rato-scratch-m4`, real `git`/`tmux`. Automated coverage: `cargo test --workspace` (incl.
`rat-workbench` runner tests + `rat-daemon` rpc tests) green.

- [x] ⚙ `rat task start --project <repo> --title <t> --adapter fakeagent` → returns a `running`
      AgentRun; a `rato/<slug>` branch + a worktree under `~/.local/share/rato/worktrees/` are created
- [x] ⚙ Docker executor command construction + RPC validation: `--executor docker --docker-image <image>`
      records runs as `docker:<image>` and builds `docker run` with the task worktree mounted at
      `/workspace`
- [x] ⚙ run advances `running → done` on its own (daemon 3 s poll sweep + poll-on-read in
      `workbench.runs`); the agent's commit lands on the `rato/*` branch, **NOT** on the live repo
      (live `HEAD` unchanged pre-merge)
- [x] ⚙ `rat task tail <run_id>` returns the captured window output
- [x] ⚙ `rat task merge-back <run_id>` → creates a **R2** `merge_back` approval (slug = last 6 of id)
      with diffstat in the payload; oversized (>32 KB) diffs go to a `blobs` row referenced by id
- [x] ⚙ `rat approvals` lists it pending; `rat approvals decide <id> approve` →
      `git merge --no-ff` lands in the **live** repo (new merge commit, agent file present),
      `agent_runs.status = merged`, and `approvals.execution` records commit sha + exit 0 + verified_target
- [x] ⚙ **denial invariant:** second run → `rat approvals decide <id> deny` → live repo
      `git rev-parse HEAD` and `git status --porcelain` **byte-identical** before/after;
      `rato/*` branch preserved (also asserted with a real diff in the `rat-workbench` deny test)
- [x] ⚙ not-fast-mergeable branch → `execute_merge` returns `NeedsManual` (no auto-resolve);
      conflicting-branch test in `rat-workbench`
- [x] ⚙ R3 slug gate exists: `approvals.decide` on an R3 approval requires a matching `--slug`
      (no R3 flows ship in M4; gate unit-tested in `rat-daemon` rpc tests)
- [x] ⚙ pending R2 approvals auto-`expired` after 60 min (daemon 60 s `expire_approvals` sweep)
- [ ] 👁 dashboard **Workbench** tab: runs table (adapter/title/status/started/diffstat), expandable
      2 s tail poll, "Merge back" button on `done` runs creates the approval
- [ ] 👁 dashboard **Approvals** tab: risk-striped cards (R1 green / R2 amber / R3 red), diffstat block,
      expiry countdown, Approve/Deny; R3 card requires typing the slug to arm Approve; audit list below
- [ ] 👁 avatar grip shows an amber `APR` chip while approvals are pending

## M5 — Eyes / screen *(in progress)*

Implementation status as of 2026-06-15: store pins, encrypted ring, trait-based vision pipeline,
daemon capture-loop seam, pins RPC/CLI, shell Pins tab, retention pruner, SensorGate health,
doctor rows, Sensors-tab ring/prune controls, and Calendar tab are landed. Live capture smoke,
24 h soak, final acceptance docs, and the `m5-eyes` tag remain.

Automated verification already green:

- [x] ⚙ `cargo test --workspace` with workspace-owned `TMPDIR`/`XDG_DATA_HOME`:
      `TMPDIR=~/rato/target/tmp XDG_DATA_HOME=~/rato/target/test-data cargo test --workspace`
- [x] ⚙ `cargo check -p rat-daemon --features screencast,ocr`
- [x] ⚙ `cd apps/shell && npm run check && npm run build`
- [x] ⚙ ring crypto: seal/open round-trip; wrong key/AAD/tamper fail; nonce uniqueness;
      fake-clock prune keeps only the bounded TTL window
- [x] ⚙ vision pipeline: fake screen + fake/null OCR; dHash dedup; OCR deltas; JPEG output;
      unavailable source returns no capture
- [x] ⚙ auto-pin regex table: panic, stack trace, Rust `error[E...]`, exception, and FAILED
      patterns match; benign text does not
- [x] ⚙ daemon capture tick with `FakeScreenSource`/`FakeOcr` writes a ring segment and inserts
      a searchable `ocr` observation through the normal observation/FTS path
- [x] ⚙ `pins.pin_recent/list/unpin` RPC round-trips over a seeded ring and removes both row
      and pin directory on unpin
- [x] ⚙ `rat pins`, `rat pins pin-recent --minutes N --media screen`, and
      `rat pins unpin <id>` are covered by CLI tests
- [x] 👁 shell Pins tab exists and can call `pins.list`, `pins.pin_recent`, and `pins.unpin`

Remaining M5 acceptance:

- [x] ⚙ retention pruner: never deletes cited observations, summaries, audit rows, or manual pins;
      deletes uncited observations older than 180 d and expired auto-pins; processes batches ≤5k
- [x] ⚙ clock-skew test: auto-pin created before a clock jump past expiry is expired and files are
      unlinked; manual pin survives
- [x] ⚙ `rat doctor` reports screen/OCR availability, ring occupancy, and pin count
- [x] ⚙ `retention.status` RPC returns `null` before first prune and persisted prune counts after
      the nightly job records them.
- [x] 👁 Sensors tab shows ring occupancy, last prune counts/time, and pin-last-N control
- [x] 👁 Calendar tab shows session timeline with events/observations/agent runs/approvals/pins
- [ ] 👁 live smoke with `--features screencast,ocr`: grant portal consent, confirm real OCR
      observations appear in `rat search`, pin last 5 min, verify pin files exist
- [ ] 👁 24 h soak: CPU <8 % average, ring bounded to 20 min, no unbounded pin/ring growth
- [ ] ⚙ tag `m5-eyes` after live smoke and 24 h soak evidence is recorded

## M6 — Voice *(in progress)*

Implementation status as of 2026-06-16: deterministic `rat-voice` core is landed (backend traits,
fake implementations, RAM-only pre-wake ring, intent router, spoken approval slug, voice-approval
gate), `voice_utterances` migration/store repo is landed as v7, daemon exposes `voice.status` and
`voice.utterances`, approval DTOs include `spoken_slug`, approval cards render it, and CLI has
`rat voice status` / `rat utterances`. The shell Settings tab now surfaces backend-aware voice
toggles/language smoke buttons, and the avatar MIC chip reflects `voice.status` with a pulse on new
post-wake utterances. Dashboard M5/M6 additive RPCs use fallbacks so newer tabs still render against
older daemons that lack `ring.status`, `retention.status`, or `voice.*`.

Automated verification already green:

- [x] ⚙ IntentRouter grammar tables cover English and Portuguese control intents, approval slug
      utterances, and fallback chat.
- [x] ⚙ Voice-approval gate: visible pending R2 + correct spoken slug allows approve/deny; hidden
      popup, R3, wrong slug, and non-pending approvals refuse.
- [x] ⚙ Two-word spoken slug is deterministic and derived from approval id without schema storage.
- [x] ⚙ Pre-wake ring is RAM-only API surface, bounded to 8 s-equivalent capacity, and zero-clears on
      explicit clear/drop.
- [x] ⚙ `voice_utterances` v7 migration preserves v6 retention data; insert/recent repo round-trips.
- [x] ⚙ `voice.status` reports default unavailable backends and `voice.utterances` returns recent
      post-wake rows.
- [x] 👁 Approvals tab renders the spoken voice slug for pending approval cards.
- [x] ⚙ `rat voice status`, `rat voice say`, and `rat utterances` CLI surfaces compile and
      integration tests pass.

Remaining M6 acceptance:

- [x] ⚙ fs-watch non-persistence test around the fake wake/VAD path and writable data/state dirs.
- [x] ⚙ daemon voice loop with FakeAudioSource/FakeWake/FakeVad/FakeStt/FakeTts records an utterance,
      routes deterministic local intents, and turns `pin that` into a manual screen-ring pin when
      a recent segment exists.
- [x] ⚙ voice approval execution path records `decided_via='voice'` plus utterance id.
- [x] 👁 Settings voice/wake toggles and language test buttons.
- [x] 👁 avatar MIC chip pulse / ear-perk wake indicator.
- [ ] 👁 live smoke with `mic,wake,stt,tts`: both languages wake and command; Piper speaks both voices.
## M7 — Polish *(in progress)*

Implementation status as of 2026-06-16: the M7 plan is written with schema v8 (v7 is already used by
M6 voice), store v8 terminals + dotfile edit audit rows are landed, `rat-terminal` has a deterministic
classifier plus best-effort real `/proc` and tmux pane collection wired into daemon storage,
`rat-inject` has the pure injection ceremony state machine,
`rat-dotfile` has the deterministic validation/atomic-apply/revert core, daemon/RPC/CLI read+revert
surfaces are wired for terminal and config-edit audit rows, and Claude/Codex adapter transcript
parsers are wired into a daemon watcher that records project-local transcript output as
`agent_output` observations.

Automated verification already green:

- [x] ⚙ v7→v8 migration preserves voice utterances and creates `terminals` / `dotfile_edits`.
- [x] ⚙ terminal upsert/list/get refreshes `last_seen`, role, tmux target, and metadata.
- [x] ⚙ dotfile edit audit rows round-trip before/after blob ids and link `reverted_by`.
- [x] ⚙ terminal classifier distinguishes RATO tmux workbench, foreign LLM terminal, remembered
      operator/ignored terminal, and non-agent processes.
- [x] ⚙ real `/proc` scanner + tmux pane mapper runs in the daemon sensor loop and upserts detected
      agent terminals while preserving remembered operator/ignored command hashes.
- [x] ⚙ injection ceremony refuses expired approvals, away mode, missing targets, changed commands,
      and X11 focus mismatch; bracketed paste and Enter are separate actions.
- [x] ⚙ DotfileEditor core rejects malformed JSON/TOML/YAML before write, rejects missing MCP
      commands, accepts JSONC comments, applies with same-directory temp+rename, and reverts exact
      bytes.
- [x] ⚙ `terminals.list` / `terminals.set_role` RPCs and `rat terminals` CLI list/classify stored
      terminal rows.
- [x] ⚙ `dotfile_edits.list` / `dotfile_edits.revert` RPCs and `rat config-edits` CLI expose the
      audit feed and byte-exact revert path with linked `reverted_by` rows.
- [x] ⚙ `dotfile_edits.apply` RPC and `rat config-edits apply` validate managed edits through
      DotfileEditor, write atomically, store before/after blobs, and reject invalid configs before
      write or audit.
- [x] 👁 `Now` tab renders first-sighting terminal dialogue cards with operator/workbench/ignore
      actions.
- [x] 👁 `Settings` tab renders a config-change feed with revert actions and diff previews.
- [x] ⚙ Claude/Codex adapter transcript dirs are project-local and JSONL parsers extract nested
      `text`/`content` fields into agent-output summaries.
- [x] ⚙ daemon transcript watcher scans recent Claude/Codex workbench runs, ingests project-local
      `.jsonl` transcript files as `agent_output` observations, and dedupes by transcript file key.

Remaining M7 acceptance:

- [ ] 👁 Metrics, Memory, Pushback, and injection card UI.
- [ ] 👁 live smoke: foreign Claude terminal detected/classified; approved tmux paste-and-enter executes.

## M8 — Hardening *(pending)*
