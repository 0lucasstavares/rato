# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder

WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates ./crates

RUN cargo build --release -p rat-daemon -p rat-cli

FROM debian:bookworm-slim AS runtime

ARG RATO_UID=1000
ARG RATO_GID=1000

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        git \
        libwayland-client0 \
        libxkbcommon0 \
        tmux \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid "${RATO_GID}" rato \
    && useradd --create-home --uid "${RATO_UID}" --gid "${RATO_GID}" --shell /usr/sbin/nologin rato \
    && mkdir -p /config /data /run/rato /state \
    && chown -R rato:rato /config /data /run/rato /state

COPY --from=builder /src/target/release/ratd /usr/local/bin/ratd
COPY --from=builder /src/target/release/rat /usr/local/bin/rat

USER rato
ENV HOME=/home/rato \
    XDG_CONFIG_HOME=/config \
    XDG_DATA_HOME=/data \
    XDG_RUNTIME_DIR=/run \
    XDG_STATE_HOME=/state \
    RAT_LOG=info

CMD ["ratd", "--socket", "/run/rato/ratd.sock", "--db", "/data/rato.db", "--no-sensors", "--no-critic"]
