# RATO

Single-user, local-first Linux developer companion. See `docs/ARCHITECTURE.md`.

## Build

    export PATH="$HOME/.cargo/bin:$PATH"
    cargo build --workspace

## Install (systemd --user)

    cargo build --release --workspace
    ./target/release/rat install
    systemctl --user status ratd

## Shell hooks (command tracking)

Add to `~/.bashrc` (or `~/.zshrc` with `zsh`):

    eval "$(~/rato/target/release/rat shell-init bash)"

or simply `source ~/rato/packaging/shell/rat-init.sh`.

## Autonomy

Autonomy runs in GitHub Actions again.

Primary workflows:

    .github/workflows/agent-scrum-master.yml
    .github/workflows/agent-manager.yml
    .github/workflows/agent-worker.yml
    .github/workflows/agent-reviewer.yml
    .github/workflows/agent-merger.yml

Control toggles:

    .github/workflows/autonomy-on.yml
    .github/workflows/autonomy-off.yml

Required repository setup:

    RATO_AUTONOMY GitHub variable = on
    ANTHROPIC_AUTH_TOKEN or RATO_CLAUDE_AUTH_TOKEN secret for Claude Code
    optional fallback: ANTHROPIC_API_KEY, OPENAI_API_KEY, or CHATGPT_API_KEY

The local supervisor remains available as a fallback operator tool. The dashboard shows GitHub Actions status and run logs:

    pwsh ./scripts/autonomy/run-local-autonomy.ps1
    node ./scripts/autonomy/dashboard-server.mjs

## Dev

    cargo test --workspace