# Running RATO

RATO is a desktop developer companion: a background daemon (`ratd`) that observes your
work through cheap sensors, a memory + critic layer that pushes back with cited evidence,
and a Tauri shell (`rato-shell`) showing a 3D rat avatar and a dashboard.

## Prerequisites

- Linux with a desktop session (tested: Ubuntu GNOME on Wayland — the shell runs via XWayland).
- System packages (one-time, needs sudo):
  `sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev pkg-config build-essential tmux`
- Rust toolchain (rustup) and Node ≥ 20 on PATH:
  `export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"`
- A Secret Service keyring (GNOME Keyring — present on stock Ubuntu).

## Build

```bash
cd ~/rato
cargo build --release                                            # daemon + CLI
(cd apps/shell && npm install && npm run build)                  # frontend
cargo build --release --manifest-path apps/shell/src-tauri/Cargo.toml   # shell binary
```

Binaries land at `target/release/{ratd,rat}` and `apps/shell/src-tauri/target/release/rato-shell`.

Optional M5 hardware backends are feature-gated. The default build is deterministic and uses
fake/null screen/OCR backends. Operator live-smoke build:

```bash
cargo build --release -p rat-daemon --features screencast,ocr
```

## First-time setup

```bash
# 1. install + start the daemon and shell as user services
cp packaging/systemd/ratd.service packaging/systemd/rato-shell.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ratd rato-shell

# 2. import LLM API keys (reads keys/antr_k.txt, open_k.txt, openr_k.txt → Secret Service;
#    never prints values) and pick the default provider
./target/release/rat setup --keys-dir ~/rato/keys --provider anthropic

# 3. restart the daemon + shell so both pick up the current binaries, then check health
systemctl --user restart ratd rato-shell
./target/release/rat doctor
./target/release/rat llm-status
```

Optional: shell-command observations come from shell hooks — add to your `~/.bashrc`:
`source ~/rato/packaging/shell/rat-init.sh`

## Configuration

`~/.config/rato/config.toml` (created with defaults on first run):

```toml
[llm]
provider = "anthropic"      # openai | anthropic | openrouter
[critic]
enabled = true
fast_tick_s = 30            # local signal detectors
slow_tick_s = 300           # LLM review cadence
```

Notes:
- Embeddings always use OpenAI (`text-embedding-3-small`). If the key is missing or the
  account rejects the model, retrieval silently degrades to full-text-only — check
  `rat llm-status` (`embedding_enabled`).
- Daemon flags: `ratd --no-sensors` (no capture), `ratd --no-critic` (no LLM calls).

## Day-to-day

| What | How |
|---|---|
| Avatar | always-on-top bottom-left; drag by the tape strip; position is remembered |
| Dashboard | double-click the rat (tabs: Now, Pushback, Workbench, Approvals, Pins, Sensors, Settings) |
| Daemon status | `rat status`, `rat doctor` |
| Events/observations | `rat events`, `rat observations [--kind shell_cmd]` |
| Search memory | `rat search "query"` |
| Pushbacks | `rat pushbacks` / `rat pushbacks feedback <id> <useful|dismiss|snooze>` |
| Workbench tasks | `rat task start --project <repo> --title <t> [--adapter fakeagent\|claude-code\|codex]`; `rat task list`; `rat task tail <run_id>` |
| Merge back | `rat task merge-back <run_id>` → creates an approval; review and approve it |
| Approvals | `rat approvals`; `rat approvals decide <id> approve\|deny [--note <n>] [--slug <s>]` |
| Capture pins | `rat pins`; `rat pins pin-recent --minutes 5 --media screen`; `rat pins unpin <id>` |
| Logs | `journalctl --user -u ratd -f` |
| Stop everything | `systemctl --user stop ratd rato-shell` |

## Workbench (agent tasks + merge-back)

A workbench task runs an agent adapter inside an isolated git worktree (branch `rato/<slug>`
under `~/.local/share/rato/worktrees/<repo-hash>/<task-id>/`) in a dedicated `tmux -L rato`
window. Agent commits stay on the `rato/*` branch — the live repo is never touched until you
approve a merge-back.

```bash
rat task start --project ~/code/myproj --title "add retry logic"   # default adapter: fakeagent
rat task list                                                      # running → done
rat task tail <run_id>                                             # captured agent output
rat task merge-back <run_id>                                       # → R2 approval (slug = last 6 of id)
rat approvals                                                      # see it pending
rat approvals decide <approval_id> approve                         # git merge --no-ff into the live repo
#   ...or: rat approvals decide <approval_id> deny                 # live repo untouched, branch kept
```

- **Adapters:** `fakeagent` (deterministic test agent), `claude-code` (`claude` on PATH),
  `codex` (`codex` on PATH). Real-adapter transcript parsing + interactive panes land in M7.
- **Merge-back is always R2** (operator approval required) and only merges when fast-forward/clean
  (`git merge-tree`); conflicts return *needs-manual* and never auto-resolve.
- **R3 approvals** (none ship in M4) require `--slug <s>` matching the approval's slug.
- Pending approvals **expire after 60 min** (daemon sweep).

## M5 Eyes (ring buffer + OCR observations + pins)

The daemon owns a 20-minute encrypted screen ring under `~/.local/state/rato/ring/`.
Default builds do not capture the real desktop: they construct fake/null screen and OCR
backends and report unavailable capability instead of fabricating observations. With live
backends enabled later, the capture loop runs every 2 s, writes 10 s ring segments, inserts
`ocr` observations for OCR deltas, and auto-pins local error/stack-trace patterns.

Manual pinning works over whatever ring segments are present:

```bash
rat pins
rat pins pin-recent --minutes 5 --media screen
rat pins unpin <pin_id>
```

Pins are copied from the ephemeral ring into `~/.local/share/rato/pins/<pin-id>/` and
re-encrypted under a persistent Secret Service key (`rato/pin-key`). Manual pins do not
expire. Auto-pins are intended to expire through the M5 retention pruner, which is the next
roadmap item.

## Data locations

| Path | Contents |
|---|---|
| `~/.local/share/rato/rato.db` | SQLite store (events, observations, memories, pushbacks, audit) |
| `~/.local/share/rato/avatar-pos.json` | remembered avatar position |
| `~/.local/share/rato/pins/` | persistent pinned ring segments |
| `~/.local/state/rato/ring/` | ephemeral encrypted capture ring |
| `/run/user/$UID/rato/ratd.sock` | daemon RPC socket (0600) |
| Secret Service `rato/openai|anthropic|openrouter|pin-key` | API keys and the persistent pin key |
