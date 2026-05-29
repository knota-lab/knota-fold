# ── Stage 1: build ─────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder
# Note: rustup installs the actual toolchain below; the base image version
# doesn't matter — we use 1.85-bookworm for Debian compatibility.
ARG RUST_VERSION=1.94.0

RUN rustup toolchain install ${RUST_VERSION} --profile minimal \
    && rustup default ${RUST_VERSION}

WORKDIR /app

# 1. Cache dependencies — copy manifests only, generate dummy source for
#    both the app crate and the migration crate so cargo can resolve and
#    compile the full dependency tree.  This layer stays cached as long as
#    Cargo.toml / Cargo.lock are unchanged.
COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml migration/Cargo.toml
RUN mkdir -p src/bin \
    && echo 'fn main(){}' > src/bin/main.rs \
    && echo 'fn main(){}' > src/bin/tool.rs \
    && echo '' > src/lib.rs \
    && mkdir -p migration/src \
    && echo '' > migration/src/lib.rs \
    && cargo build --release

# 2. Build application — copy real source, migrations, fixtures, config.
#    cargo clean -p removes only app/migration crate artifacts (deps cached),
#    then rebuilds with real source.
COPY migration/src migration/src/
COPY src/fixtures src/fixtures/
COPY src src/
COPY config config/
RUN cargo clean -p knota-fold -p migration --release \
    && cargo build --release --bin knota_fold-cli \
    && cp target/release/knota_fold-cli /tmp/knota_fold-cli

# ── Stage 2: minimal runtime ──────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates netcat-openbsd \
    && groupadd --system appuser \
    && useradd --system --gid appuser --create-home appuser \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /tmp/knota_fold-cli /usr/local/bin/knota_fold-cli
COPY --from=builder /app/config ./config
COPY --from=builder /app/migration ./migration
COPY --from=builder /app/src/fixtures ./src/fixtures

# Pre-create data directories with correct ownership so named volumes
# inherit appuser permissions on first mount.
RUN mkdir -p /app/data/logs \
    && chown -R appuser:appuser /app

ENV LOCO_ENV=production \
    PORT=5150 \
    RUST_LOG=info

EXPOSE 5150

HEALTHCHECK --interval=30s --timeout=3s --start-period=20s --retries=3 \
    CMD nc -z 127.0.0.1 5150 || exit 1

USER appuser

ENTRYPOINT ["/usr/local/bin/knota_fold-cli", "start", "--all"]
