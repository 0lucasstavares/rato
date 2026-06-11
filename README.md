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

## Dev

    cargo test --workspace
