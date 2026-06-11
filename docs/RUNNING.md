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

## First-time setup

```bash
# 1. install + start the daemon and shell as user services
cp packaging/systemd/ratd.service packaging/systemd/rato-shell.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ratd rato-shell

# 2. import LLM API keys (reads keys/antr_k.txt, open_k.txt, openr_k.txt → Secret Service;
#    never prints values) and pick the default provider
./target/release/rat setup --keys-dir ~/rato/keys --provider anthropic

# 3. restart the daemon so it picks the keys up, then check health
systemctl --user restart ratd
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
| Dashboard | double-click the rat (tabs: Now, Pushback, Sensors, Settings) |
| Daemon status | `rat status`, `rat doctor` |
| Events/observations | `rat events`, `rat observations [--kind shell_cmd]` |
| Search memory | `rat search "query"` |
| Pushbacks | `rat pushbacks` / `rat pushbacks feedback <id> <useful|dismiss|snooze>` |
| Logs | `journalctl --user -u ratd -f` |
| Stop everything | `systemctl --user stop ratd rato-shell` |

## Data locations

| Path | Contents |
|---|---|
| `~/.local/share/rato/rato.db` | SQLite store (events, observations, memories, pushbacks, audit) |
| `~/.local/share/rato/avatar-pos.json` | remembered avatar position |
| `/run/user/$UID/rato/ratd.sock` | daemon RPC socket (0600) |
| Secret Service `rato/openai|anthropic|openrouter` | API keys |
