export PATH := env_var("HOME") + "/.cargo/bin:" + env_var("PATH")

default: test

build:
    cargo build --workspace

test:
    cargo test --workspace

release:
    cargo build --release --workspace

install: release
    ./target/release/rat install
