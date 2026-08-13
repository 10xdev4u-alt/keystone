# syntax=docker/dockerfile:1
# Multi-stage build: compile once in a full toolchain, ship a distroless
# runtime (~40 MB — no shell, no package manager, just glibc + CA certs).

###############################################################################
# Builder — compile the API in release mode with maximal layer caching.
###############################################################################
FROM rust:1-bookworm AS builder

WORKDIR /build

# 1. Cache dependencies: copy manifests only, fetch, then add source.
#    Any dependency change busts this layer; source edits reuse it.
COPY Cargo.toml Cargo.lock ./
COPY crates/api/Cargo.toml crates/api/Cargo.toml
COPY crates/auth/Cargo.toml crates/auth/Cargo.toml
COPY crates/config/Cargo.toml crates/config/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/domain/Cargo.toml crates/domain/Cargo.toml

# Skeleton so `cargo fetch` resolves the workspace, then drop the skeletons.
RUN mkdir -p crates/api/src crates/auth/src crates/config/src crates/db/src crates/domain/src \
    && for c in api auth config db domain; do \
         echo 'fn main() {}' > crates/$c/src/main.rs; \
         echo '' > crates/$c/src/lib.rs; \
       done \
    && cargo fetch

COPY crates crates

# Build only the API binary (skips the dump-openapi helper).
RUN cargo build --release -p keystone-api \
    && strip target/release/keystone-api

###############################################################################
# Runtime — distroless: glibc + CA certs (S3/TLS egress), non-root, no shell.
###############################################################################
FROM gcr.io/distroless/cc-debian12 AS runtime

WORKDIR /app
COPY --from=builder /build/target/release/keystone-api /usr/local/bin/keystone-api

# distroless ships a nonroot user (65532) and default-deny everything else.
USER nonroot
EXPOSE 4000

ENTRYPOINT ["/usr/local/bin/keystone-api"]
