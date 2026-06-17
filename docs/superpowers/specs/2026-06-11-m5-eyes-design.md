# M5 — Eyes (screen capture / ring buffer / OCR / pins / retention) design

**Date:** 2026-06-11
**Status:** approved (autonomous-goal mode; decisions from ARCHITECTURE.md §5, §10, §12, §14, §18-M5, §19)
**Acceptance (§18):** 24 h soak CPU <8 % avg, ring bounded at 20 min, OCR observations searchable,
pins expire correctly (clock-skewed test).

## Reality constraints (binding context for the decisions below)

The operator is not present (autonomous mode), so no `sudo apt install` and no interactive portal
consent can happen this milestone. Screen capture (xdg-desktop-portal ScreenCast → PipeWire),
OCR (tesseract system libs), and a literal 24 h soak are all environment/time/privilege bound.
Therefore M5 lands the **deterministic, testable core** behind seams, with the hardware-bound
backends as runtime-detected/feature-gated implementations that **degrade gracefully** when their
platform support is absent — exactly the §5 SensorGate "health" model. Nothing fabricates capability
it can't deliver; `rat doctor` reports what's actually available.

## Decisions (autonomous defaults)

| Question | Decision | Why |
|---|---|---|
| Screen capture backend | `ScreenSource` trait. Real impl `PortalScreenSource` (ashpd → xdg-desktop-portal ScreenCast, persistent restore token in keyring `rato/screencast-token`, PipeWire frame grab). Behind `--features screencast`; if the feature is off OR portal/PipeWire unavailable at runtime → source reports `Unavailable` and the screen sensor sits in SensorGate state `unavailable` (not an error). A `FakeScreenSource` (scripted frames) drives all tests. | The capture stack can't run headless/without consent; the trait lets the pipeline, ring, OCR, pins, and pruner be fully built and tested now, with the real backend swapping in when the desktop supports it. |
| Frame cadence + dedup | grab every 2 s; skip if dHash (64-bit) Hamming distance ≤4 from last kept frame (per §5). | §5 verbatim; cheap, deterministic, unit-testable on fixture images. |
| OCR engine | `OcrEngine` trait. Real impl `TesseractOcr` behind `--features ocr` (via `leptess`/`rusty-tesseract` — pick whichever builds without extra system deps beyond libtesseract; if neither builds cleanly, gate it and ship the trait + fake only). Default build = no OCR feature → `NullOcr` returns empty + SensorGate marks OCR `unavailable`. `FakeOcr` (returns scripted text) drives tests. OCR output diffed against the previous frame's OCR; only changed blocks become observations. | tesseract needs sudo libs the operator must install; degrade-not-fail keeps M5 shippable and the pipeline correct. |
| Ring buffer crypto | `~/.local/state/rato/ring/{screen,audio,clipboard}/`; fixed-duration segments; XChaCha20-Poly1305 (`chacha20poly1305` crate) with an **ephemeral per-run key** generated at daemon start, held in an `mlock`ed buffer (`region`/`memsec` or a minimal mlock wrapper; if mlock unavailable, still ephemeral + zeroize on drop and log a doctor warning). Writer prunes segments older than 20 min every 60 s. | §5 verbatim. Ephemeral key = crash makes old ring unreadable (a privacy feature). Audio/clipboard dirs are created but only screen is wired this milestone (audio is M6, clipboard ring is a thin add). |
| Ring segment length | 10 s segments (§5). | verbatim. |
| Pins | store migration **v5**: `pins(id PK, kind auto|manual, media screen|audio|clipboard, path, created, expires_at NULL, reason, meta JSON)` per §10. `pin(media, ring_segment_ids, reason, kind)` re-encrypts the named ring segments with a **persistent pin key** (keyring `rato/pin-key`, created on first use) and writes them under `~/.local/share/rato/pins/<id>/`; auto-pin `expires_at = now+30d`, manual `NULL`. `unpin(id)`, `list_pins`, `expire_pins(now)`. | §5 + §10. Persistent key so pins survive restarts (unlike the ring). |
| Auto-pin classifier | local regex pre-filter only this milestone (stack-trace/panic/error-dialog patterns over OCR deltas) → auto-pin with `reason`. The LLM cheap-model classifier (§5) is **deferred**: the operator's OpenAI key is model-restricted (gpt-5-mini 403, see M3) and anthropic-cheap-model classification of every frame is out of scope/cost. Manual "pin last N minutes" is the primary path. | Can't depend on a blocked model; regex pre-filter is the §5-specified cheap gate and is deterministic/testable. |
| Retention pruner | nightly 03:30 maintenance job: citation-aware per §12 — observations >180 d delete unless cited by a memory/pin/summary (+ their FTS/vec rows); auto-pins past `expires_at` hard-delete (file + row, logged); manual pins never; audit tables exempt. `DELETE` batches ≤5 k rows; emits a prune-count event; "last prune" surfaced in Sensors tab and `retention.status`. | Runs even when critic/LLM is disabled; LLM day summaries are skipped without a backend, but retention/decay still execute. Fully fake-clock + invariant testable (§19). |
| Soak/CPU acceptance | documented manual procedure in TEST-CHECKLIST (operator-run when capture is live); automated substitute = fake-clock tests proving ring stays bounded (segment count ≤ 20min/10s window across simulated time) and a frame-rate/dedup unit test bounding work per tick. No literal 24 h run this session. | the soak is hardware+time bound; the invariants it checks are testable deterministically. |
| Calendar tab | implement the §14 Calendar timeline reading work_sessions + per-block glyphs (commands, test pass/fail, agent runs, approvals, pins, stuck loops, ctx switches) from existing tables; day/week zoom; row→session detail. Read-only (no creation UI, §14). | §18-M5 deliverable. All source data already exists in the store. |
| Sensors tab additions | ring-buffer occupancy meter, prune-log line, "pin last N minutes" button (calls `pins.pin_recent {minutes}`). | §14 Sensors row + §5 pin path. |

## Components

1. **store migration v5**: `pins` table per §10 + indexes; PinRepo (`insert_pin`/`list_pins`/`get_pin`/
   `expire_pins(now)->u32`/`delete_pin`). Additive v4→v5, user_version 5.
2. **`rat-ring` (module in rat-sensors or small crate)**: `RingKey` (ephemeral, mlock+zeroize),
   `RingWriter { dir, segment_secs, ttl_secs, clock }` (`write_frame`/`prune(now)`/`list_segments`/
   `read_segment` for pinning), XChaCha20-Poly1305 seal/open. Fake-clock rotation tests.
3. **`ScreenSource` trait + `dhash` dedup + capture loop** in rat-sensors: `PortalScreenSource`
   (feature `screencast`), `FakeScreenSource`. Loop: grab→dHash skip→ring write→OCR derive.
4. **`OcrEngine` trait**: `TesseractOcr` (feature `ocr`), `NullOcr`, `FakeOcr`. OCR-delta →
   `observations(kind=ocr, content, meta={window_title,...})` → existing FTS + embed queue.
   Local regex auto-pin pre-filter.
5. **Pin service**: re-encrypt ring segments under persistent pin key → `pins/`; `pins.pin_recent`,
   `pins.list`, `pins.unpin` RPC + `rat pins [pin-recent N | list | unpin <id>]` CLI.
6. **Retention pruner**: nightly job, citation-aware, fake-clock + proptest invariants (§19:
   never delete cited observations / audit rows / manual pins; always delete expired auto-pins).
7. **SensorGate**: add `screen`/`ocr` health states incl. `unavailable`; `rat doctor` reports them.
8. **Shell**: Calendar tab (§14) + Sensors-tab ring occupancy/prune-log/"pin last N min".

## Testing (§19)

Unit/integration (all deterministic, no hardware): ring rotation + crypto round-trip under fake clock
(segments bounded to 20 min window; sealed segment unreadable with a different key); dHash dedup on
fixture images; OCR-delta → observation via `FakeOcr`; regex auto-pin pre-filter; pin re-encrypt +
`expire_pins` under clock skew (auto-pin past expiry gone, manual survives); pruner invariants via
proptest; Calendar data assembly from a synthetic session timeline; migration v4→v5 on a populated db.
Live smoke (operator, when desktop allows): `--features screencast,ocr` build, grant portal consent
once, confirm OCR observations appear in `rat search`, pin last 5 min → pin file exists, expires.

## Out of scope (deferred)

Audio ring + STT (M6), LLM auto-pin classifier (until a usable cheap model), tmux/foreign-pane output
summaries as a sensor (§5 row; M7 terminal work), literal 24 h soak (operator procedure), Memory-tab
pins gallery polish (M7).
