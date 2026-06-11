# M8 — Hardening (threat-model pass / Aider+Gemini / soak / backup-restore / cleanup jobs) design

**Date:** 2026-06-11
**Status:** approved (autonomous-goal mode; decisions from ARCHITECTURE.md §17, §7, §12, §18-M8, §19)
**Acceptance (§18):** 7-day soak no leaks (RSS flat ±10 %); all §19 suites green.

## Reality constraints (binding)

Operator absent (no sudo, no 7-day wall-clock run this session). M8 lands the **verifiable hardening
artifacts** — a threat-model checklist with a test/audit backing each row, the remaining adapters,
backup/restore, the deferred cleanup jobs, the tmux control-mode client — and converts the soak into
(a) deterministic invariant/leak tests runnable now and (b) a documented operator soak procedure with
instrumentation. "No leaks" is asserted by bounded-resource invariant tests + a memory-growth harness
over simulated load, not a literal 7-day run.

## Decisions (autonomous defaults)

| Question | Decision | Why |
|---|---|---|
| Threat-model checklist | turn each §17 row into a CHECKLIST item in `docs/TEST-CHECKLIST.md` M8 section, each backed by either an automated test or a documented audit step: dir/socket/db perms (test), ring/pin encryption (test), prompt-injection containment (test: observation→UNTRUSTED fence + schema-only proposals + policy independent of model claims), MCP/install R3 gating (test), injection wrong-target JIT (M7 state-machine tests), secret-pattern blocking (test), strict-serde/size-caps (test), approval-fatigue scoping (exception store test), Away-mode blocks (test), egress allowlist (audit + a connection-attempt test against the configured providers only). | §17 + §18; "checklist pass" must mean evidence, not assertion. |
| Egress control | add an egress allowlist check in `rat-brain`'s HTTP layer: outbound base URLs restricted to configured provider endpoints (`api.openai.com`/`api.anthropic.com`/`openrouter.ai`) + a documented note that adapters run as the user (out of scope to proxy). `store:false` on OpenAI calls (verify present). Unit-test the allowlist rejects others. | §17 egress row. |
| Secret-pattern classifier | implement the §17 secret-like clipboard/OCR redaction: regex classifier (API keys, tokens, password-like, private keys) that BLOCKS derivation/embedding of secret-like clipboard entries (ring-only) and runs an OCR redaction pass before storage. Table-driven tests. | §17 secrets row; partially specified since M1 (clipboard classify) — complete it. |
| Aider + Gemini adapters | implement `AgentAdapter` for `aider` (`aider --message "<task>" --yes` headless) and `gemini` per their CLIs, + transcript parsers (`.aider.*`, Gemini dirs), binary-detected, headless. Same trait as M4/M7. | §18-M8; mechanical given the M4 trait + M7 parser pattern. |
| Backup / restore | implement the §12 nightly `VACUUM INTO` backup (7 rotating, `~/.local/share/rato/backups/rato-<n>.db`) in the nightly job + a `rat backup [--now]` and `rat restore <path>` CLI (restore stops writes, swaps the db, restarts the store actor). Tests: backup produces a valid openable db; restore round-trip preserves data. | §12 backup row + acceptance "backup/restore". |
| Deferred cleanup jobs | worktree 14-day inactivity cleanup (§7: `git worktree remove` + branch→bundle blob kept 30 d, dashboard notice) as a nightly job; auto-pin 30 d already in M5 pruner; API-call-log 365 d prune. All fake-clock tested. | §7/§12 deferrals from M4/M5. |
| tmux control-mode client | add the §7 control-mode client (`tmux -C -L rato attach`) consuming `%output`/`%window-add`/`%session-changed` for live workbench pane summaries → `observations(kind=agent_output)` / Workbench live tail. Behind the existing tmux availability; parser is pure + fixture-tested over recorded control-mode output. | §7 deferral from M4; the parser is testable on captured streams. |
| Soak | document the operator 7-day procedure (run with all features, instrument RSS via the existing metrics, watch ring occupancy + prune logs + false-accept counter). Automated substitute: a leak/bounded-resource harness — drive ring writer + observation insert + pruner over heavy simulated load with a fake clock and assert segment/row counts stay bounded and no monotonic unbounded growth in tracked structures; ring stays ≤20 min; pruner keeps DB bounded. | acceptance soak is time-bound; the invariants it protects are testable now. |
| §19 suite completeness | ensure every §19 suite exists & is green: PolicyEngine table (M4 ✓ — extend with mode/exception axes), sessionizer golden/insta, pruner proptest (M5), retrieval-fusion determinism, language detection, ring rotation (M5), injection ceremony (M7), nextest config. Fill gaps. | §18 "all §19 suites green". |

## Components

1. egress allowlist + secret-pattern classifier/redaction (rat-brain / rat-sensors) + tests.
2. Aider + Gemini adapters + transcript parsers.
3. backup/restore (nightly `VACUUM INTO` ×7) + `rat backup`/`rat restore`.
4. nightly cleanup jobs: worktree 14-day (+ branch bundle blob), api_calls 365-day; fake-clock tests.
5. tmux control-mode client + control-stream parser (fixture-tested) → live Workbench pane summaries.
6. policy-exception store + Settings UI (standing scoped exceptions, visible/revocable; §11) — closes the
   M4-deferred exceptions/scopes; ActionKind+pattern (exact argv prefix, no regex) match, audit-logged.
7. leak/bounded-resource harness + threat-model checklist (TEST-CHECKLIST M8) with per-row evidence.
8. final docs: RUNNING.md (backup/restore, feature builds, all CLI), TEST-CHECKLIST M5–M8 complete, handoff.

## Testing (§19)

Egress allowlist rejects non-providers; secret classifier table (keys/tokens/passwords blocked, normal
text passes); OCR redaction masks key formats; Aider/Gemini adapter cmd construction + transcript parse
fixtures; backup→openable db + restore round-trip preserves data (incl. mid-write restore safety);
worktree cleanup fake-clock (14-day inactive removed, branch bundle kept, active untouched); api_calls
365-day prune; control-mode parser over recorded `%output`/`%window-add` streams; policy-exception match
(exact prefix only, revoke works, audit row written); leak harness (bounded counts under simulated load).
Full `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` + nextest +
shell `npm run check`+`build` all green. Operator: 7-day soak per documented procedure (RSS ±10 %).

## Out of scope (deferred / never)

sudo anything (refused by policy, never), sandboxing trusted dev-tool adapters' own egress (§17 documented
out-of-scope), multi-user adversarial hardening (single-trusted-user product), full art polish.
