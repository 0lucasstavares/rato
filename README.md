# RATO

Single-user, local-first Linux developer companion. See `docs/ARCHITECTURE.md`.

## Build

    export PATH="$HOME/.cargo/bin:$PATH"
    cargo build --workspace

## Install (systemd --user)

    cargo build --release --workspace
    ./target/release/rat install
    systemctl --user status ratd

## Dev

    cargo test --workspace
