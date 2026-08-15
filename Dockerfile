# Multi-stage build for DNA Storage web server + CLI

# ─── Builder ───────────────────────────────────────────────
# Note: MSRV >= 1.87 (le code utilise des APIs stabilisées en 1.87,
# ex. usize::div_ceil sur slices) — garder une image >= 1.87.
FROM rust:1.87-slim AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# Build release binaries
RUN cargo build --release -p adn-cli -p adn-web

# ─── Runtime ───────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd --system --create-home --shell /usr/sbin/nologin adn

WORKDIR /app

# Copy binaries
COPY --from=builder /build/target/release/adn /usr/local/bin/adn
COPY --from=builder /build/target/release/adn-web /usr/local/bin/adn-web

# Copy web assets (templates, static files) and config
COPY crates/web/templates/ ./templates/
COPY crates/web/static/ ./static/
COPY config.toml ./

# Dossier de travail des résultats d'encodage/décodage, accessible à adn
RUN mkdir -p /app/uploads && chown -R adn:adn /app
USER adn

# Le conteneur doit écouter sur 0.0.0.0 pour être joignable via -p.
# config.toml (127.0.0.1) est surchargé ici.
ENV ADN_HOST=0.0.0.0

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

# Default: run the web server.
# Alternative (CLI): docker run --rm --entrypoint adn <image> encode --input /data/file.txt --output /data/out.fasta
ENTRYPOINT ["adn-web"]
