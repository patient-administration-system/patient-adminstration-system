# syntax=docker/dockerfile:1
#
# Multi-stage production build for the PAS server.
#
# Stage 1 (builder): compiles the `patient-administration-system` binary in
# release mode against a slim Rust toolchain. Dependency layers are cached by
# building a stub `src/main.rs` against `Cargo.toml` / `Cargo.lock` before
# copying the real source tree.
#
# Stage 2 (runtime): minimal Debian image carrying only the binary, TLS roots,
# libssl, and curl (for the HTTP healthcheck). Runs as a non-root user.

FROM rust:1-slim AS builder
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies: copy manifests and build a stub binary so Cargo resolves
# and compiles the dependency graph into a layer that only invalidates when
# Cargo.toml / Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
COPY migrations/Cargo.toml ./migrations/Cargo.toml
RUN mkdir src \
    && echo 'fn main(){}' > src/main.rs \
    && cargo build --release --bin patient-administration-system || true
RUN rm -rf src

# Real build
COPY src ./src
COPY benches ./benches
COPY migrations ./migrations
RUN cargo build --release --bin patient-administration-system

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 pas
USER pas
WORKDIR /app
COPY --from=builder /app/target/release/patient-administration-system /usr/local/bin/pas
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://localhost:8080/api/health || exit 1
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/pas"]
