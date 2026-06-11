# M7 — Polish (avatar / terminal detection+injection / DotfileEditor+MCP / remaining tabs) design

**Date:** 2026-06-11
**Status:** approved (autonomous-goal mode; decisions from ARCHITECTURE.md §13, §8, §16, §14, §6, §18-M7, §19)
**Acceptance (§18):** foreign Claude terminal detected & classified; approved paste-and-enter executes
with ceremony; `.claude` edit auto-applies with revert.

## Reality constraints (binding)

Operator absent (no sudo, no live desktop session to drive injection/focus, no GPU). Injection targets,
window focus, and the full PS2 avatar render are environment-bound. M7 lands the **deterministic logic**
(detection classifier, injection-ceremony state machine, DotfileEditor chokepoint, transcript parsers,
tab data assembly) behind seams with fakes; the desktop/render-bound backends (xdotool/portal/ydotool
injection, tmux paste, the glTF avatar + shaders) are feature/runtime-gated and operator-smoke-verified.
Injection NEVER fires without the full ceremony + JIT recheck — proven by tests, not trusted at runtime.

## Decisions (autonomous defaults)

| Question | Decision | Why |
|---|---|---|
| Terminal detection | `rat-terminal` crate: procfs scan (every 10 s) for adapter binary names (`claude`/`codex`/`aider`/`gemini`+configured), resolve TTY via `/proc/<pid>/fd`, map to tmux pane via `tmux list-panes -a -F`, walk parent chain for emulator. Pure-logic classifier over an injected `ProcSource` (real /proc impl + fake fixtures). Classify rato-workbench (our `-L rato` socket) vs foreign. Store in `terminals` (migration **v7**) with role. | §8; classifier is testable on fixtures; first-sighting dialogue is shell-side. |
| Terminal first-sighting | foreign LLM terminal → avatar DialogueBox: Operator-terminal (observe transcripts only) / Make-it-a-workbench / Ignore (remembered per tty+cmd hash). Role persisted. | §8 verbatim. |
| Injection paths | `Injector` trait with strict preference order per env: (1) tmux `load-buffer`+`paste-buffer -p`+separate `send-keys Enter`, (2) XDG portal RemoteDesktop/libei (Wayland), (3) ydotool (opt-in uinput), (4) xdotool/XTEST (X11). Real impls feature/runtime-gated; `FakeInjector` records calls. Path selection logic is pure + tested. | §8 table verbatim; tmux path is the one we can actually exercise (M4 already shells out to tmux). |
| Injection ceremony | state machine (pure, exhaustively tested): R2 approval renders exact bytes (monospace, structured — never model-authored markup), target, whether Enter included, expiry (10 min). JIT recheck at execute: pane/window exists, `pane_current_command` matches recorded, (X11) focused window == target; (Wayland, focus unverifiable) 3 s countdown overlay + abort on any input. Result appended to approval. Away mode hard-blocks regardless of approval. Bracketed paste always; Enter is a separate approved keystroke. | §8 verbatim; this is the security spine — model it as a typed state machine so illegal transitions can't compile/execute. |
| DotfileEditor | `rat-dotfile` single chokepoint for EVERY managed write: read→`before_blob` (sha-addressed in `blobs`), compute edit, validate (parse JSON/JSONC/TOML/YAML per type, schema-check known keys, MCP command exists on `$PATH`/abs path), apply atomically (temp+rename)→`after_blob`+diff+reason+source, emit event → Approvals "Config Changes" feed with diff + one-click revert (revert = new edit writing before_blob, linked `reverted_by`). Store migration **v7** also adds `dotfile_edits` per §10. Validation failure → abort, never half-apply. | §16 verbatim; the chokepoint invariant is the safety property. |
| Policy mapping | `.claude`/`.agents` + MCP edits referencing already-installed local binaries = **R1** (auto, audited, reversible); MCP needing download/global install = **R3** (full provenance card); shell-startup edits = **R3** always. Wire to `rat-policy` ActionKinds (DotfileEditManaged R1, McpKnownEdit R1, McpNewBinary R2/GlobalInstall R3, ShellStartupEdit R3 — already in the table). | §16 + §11; tiers already exist from M4. |
| Config indexing | read the known config surface (§16 list) as R0 → `observations(kind=note, meta=config)` so the critic can cite agent setup. | §16. |
| Real-adapter transcripts | implement `parse_transcript`/`transcript_dirs` for ClaudeCode (`~/.claude/projects/**.jsonl`), Codex, (Aider/Gemini stubs→M8) — parse turns → `agent_output` observations. Replaces the M4 stubs. Watched via the existing sensor pattern. | §5 LLM-transcript row + M4 deferral. |
| Avatar model | full glTF rat (~2,200 tris, 22 bones, clips idle/alert/point/judge/typing/talk/sleep/blindfold) + PS2 shaders (vertex snap, affine UV wobble, 4-bit dither post) behind a runtime asset load; if assets absent, keep the current placeholder avatar (no regression). Personality modes (Mentor/Chaos/Quiet/Hype/Rubber-Duck) drive clip+tone+voice params ONLY (§11 invariant). | §13; model authoring is an art-asset task — ship the rig + shader + mode wiring, asset can land separately. |
| Remaining tabs | complete Metrics (game meters/sparklines from existing tables), Memory (notes browser + pins gallery + disclosure ledger), Pushback (filters + acceptance sparkline) per §14. Read-only assembly from existing store data; additive read RPCs only if needed. | §14, §18-M7 deliverable. |

## Components

1. store migration **v7**: `terminals` + `dotfile_edits` tables + repos.
2. `rat-terminal` crate: ProcSource trait (+fake), detection/classification, tmux pane mapping.
3. `rat-inject` (or in rat-terminal): `Injector` trait (+FakeInjector + feature-gated tmux/portal/ydotool/xdotool),
   injection-ceremony state machine + JIT recheck (pure, exhaustively tested), Away-mode hard block.
4. `rat-dotfile` crate: validating atomic chokepoint + revert; config-surface indexer.
5. adapters: real `parse_transcript`/`transcript_dirs` (claude/codex) → `agent_output` observations.
6. avatar: glTF rig + PS2 shaders + personality-mode wiring (graceful placeholder fallback).
7. shell: Metrics/Memory/Pushback tabs complete; Approvals "Config Changes" feed + revert; terminal
   first-sighting DialogueBox; injection ApprovalCard with exact-bytes render + countdown overlay.
8. daemon/CLI: RPCs for terminals (list/classify/role), dotfile edits (feed/revert), injection approvals;
   `rat terminals`, `rat config-edits [revert <id>]`.

## Testing (§19)

Deterministic: detection classifier over /proc + tmux fixtures (rato vs foreign vs operator); injection
path-selection logic per env; **injection ceremony state machine — exhaustive: no execute without an
approved+unexpired approval, JIT mismatch aborts, Away blocks even when approved, Enter separate from
paste** (security-critical); DotfileEditor — validation rejects unparseable/unknown-key/missing-binary,
atomic apply, before/after blobs, revert restores byte-identical + links reverted_by, never half-applies
on validation failure (proptest over random edits); transcript parsers on captured `.jsonl` fixtures →
expected `agent_output` rows; tab data assembly from synthetic store. Frontend `npm run check`+`build`.
Operator live-smoke: foreign `claude` terminal detected+classified; approved tmux paste-and-enter executes
with countdown; a `.claude/settings.json` edit auto-applies (R1) and one-click reverts.

## Out of scope (deferred)

Aider/Gemini adapters + transcript parsers (M8), tmux control-mode event-stream client (M8), final art
texturing/animation polish beyond the rig+shader, ydotool uinput setup automation (doctor-guided only).
