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

## M4 — Workbench *(pending)*
## M5 — Eyes / screen *(pending)*
## M6 — Voice *(pending)*
## M7 — Polish *(pending)*
## M8 — Hardening *(pending)*
