FROM rust:1.98.0-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY tools ./tools

RUN cargo build --release --locked -p betterlite-lens-worker && \
    install -m 0755 target/release/betterlite-lens-worker /usr/local/bin/betterlite-lens-worker

FROM cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d

COPY --from=builder /usr/local/bin/betterlite-lens-worker /usr/local/bin/betterlite-lens-worker

USER 65532:65532

ENTRYPOINT ["/usr/local/bin/betterlite-lens-worker"]
