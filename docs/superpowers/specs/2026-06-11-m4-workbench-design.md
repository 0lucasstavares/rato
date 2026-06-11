# M4 — Workbench design

**Date:** 2026-06-11
**Status:** approved (autonomous-goal mode; decisions from ARCHITECTURE.md §7, §11, §18-M4, §19)
**Acceptance (§18):** agent completes a scripted task in a worktree; merge-back requires
approval and lands clean; denial leaves live repo untouched (asserted by test).

## Decisions (autonomous defaults)

| Question | Decision | Why |
|---|---|---|
| tmux integration depth | shell-out commands (`tmux -L rato ...`) + `capture-pane` polling for tails; NO control-mode client in M4 | control-mode event stream is plumbing the acceptance doesn't need; documented deviation from §7, revisit in M8 |
| Agent adapters shipped | trait + `fakeagent` (deterministic test binary, per §19) + `claude-code` + `codex` (binary-detected, headless only) | acceptance runs on fakeagent; real adapters land but interactive panes defer to M7's terminal work |
| Approval surfaces | CLI (`rat approvals`), dashboard Approvals tab + Workbench tab; ApprovalCard popup window deferred (avatar bubble links to dashboard) | popups are M7 polish, same as pushbacks |
| PolicyEngine scope | new `rat-policy` crate, table-driven R0–R3 per §11, consumed by merge-back (R2) and workbench command gating; exceptions/scopes (this-session/pattern) deferred to M8 | M4 needs the tier table + approval gates, not the full exception store |
| R3 typed-slug confirmation | implemented in CLI/dashboard for R3 approvals (no R3 flows ship in M4, but the gate exists) | cheap now, required later |

## Components

1. **Store migration v4**: `approvals`, `actions`, `agent_runs`, `blobs` tables per §10 DDL
   (terminals/dotfile_edits defer to M7). Repos: ApprovalRepo (insert/pending/decide/expire,
   execution result append), AgentRunRepo (insert/update status/list), BlobRepo (sha256 dedup).
2. **`rat-policy` crate**: `risk_tier(ActionKind, ctx) -> R0|R1|R2|R3` table per §11;
   `requires_approval(tier)`; unit-tested table. Hard invariant: no API to mutate tiers.
3. **`rat-workbench` crate**:
   - `Tmux` wrapper: ensure server (`-L rato`), session per project (`rato-<slug>`),
     window per task (`t<id>-<slug>`), `run_in_window`, `capture_tail(target, lines)`,
     `kill_window`. All via Command shell-out; integration-tested against a throwaway
     `-L rato-test` server.
   - `Worktrees`: `create(repo, task_id, slug, base) -> WorktreePath` at
     `~/.local/share/rato/worktrees/<repo-hash>/<task-id>/`, branch `rato/<slug>`;
     `diffstat(worktree, base)`, `full_diff`, `remove`. Env scrub on agent spawn
     (no SSH_AUTH_SOCK). Resolved-path escape guard: any command cwd outside the
     worktree root is refused.
   - `AgentAdapter` trait per §6: `name`, `detect_binary`, `headless_cmd(task, worktree)`,
     `parse_transcript`, `health`. Implementations: `fakeagent` (runs
     `tests/fixtures/fakeagent.sh` — scriptable via env: writes files, commits, exits),
     `claude-code` (`claude -p "<task>" --output-format json` in worktree), `codex`
     (`codex exec "<task>"`). Headless runs execute inside the task's tmux window.
   - `TaskRunner`: `start_task(project, title, adapter, base) -> agent_run` (worktree +
     tmux window + spawn), `poll(run_id)` (process/window status → running/done/failed),
     `merge_back(run_id) -> ApprovalRequest(R2)` with diffstat+diff in payload;
     on approval: in the LIVE repo `git merge --no-ff rato/<slug>` only if fast-mergeable
     (`git merge-tree` clean) else status `needs_manual`; denial → branch + worktree kept,
     live repo untouched.
4. **Daemon + proto + CLI**: RPC `workbench.start {project_id, title, adapter}`,
   `workbench.runs {n}`, `workbench.tail {run_id, lines}`, `approvals.pending`,
   `approvals.decide {id, approve|deny, note?, slug?}`. CLI: `rat task start|list|tail`,
   `rat approvals [decide <id> approve|deny]`. Approval expiry: pending R2 expire after
   60 min (checked lazily on read + a 60s sweep in the daemon).
5. **Shell**: **Workbench tab** (runs table w/ status meters, per-run diffstat, tail view
   poll 2s, Merge-back button → creates approval; Approve/Deny inline when pending) and
   **Approvals tab** (pending queue as risk-striped ApprovalCards — green/amber/red border
   per tier, full audit list below; R3 cards require typing the slug to enable Approve).
   Avatar: when approvals pending > 0, NET-style chip strip gains an amber `APR` chip.

## Testing

Unit: policy table; worktree path hashing; adapter cmd construction; approval state machine
(pending→approved→executed / denied / expired).
Integration (per §19): throwaway git repo + `tmux -L rato-test`: fakeagent task →
worktree created, branch `rato/*`, commit lands in worktree, NOT in live repo; merge-back
approval → merge --no-ff in live repo, tree hash changes as expected; **denial test:
live-repo tree hash byte-identical before/after**; worktree-escape guard refuses
`cwd=/tmp`; expiry transitions.
Live acceptance: real run of the above against a scratch repo on the operator's machine via
CLI + dashboard, then tag `m4-workbench`.

## Out of scope (deferred)

Interactive agent panes + terminal detection/injection (M7), approval popup windows (M7),
control-mode tmux client (M8), Aider/Gemini adapters (M8), policy exceptions UI (M8),
worktree 14-day cleanup job (M8).
