# Multi-stage build for DNA Storage web server + CLI

# ─── Builder ───────────────────────────────────────────────
FROM rust:1.82-slim AS builder

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
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binaries
COPY --from=builder /build/target/release/adn /usr/local/bin/adn
COPY --from=builder /build/target/release/adn-web /usr/local/bin/adn-web

# Copy web assets (templates, static files) and config
COPY crates/web/templates/ ./templates/
COPY crates/web/static/ ./static/
COPY config.toml ./

# Default: run the web server
EXPOSE 8080

CMD ["adn-web"]

# Alternative: use the CLI
# docker run --rm dna-storage adn encode --input /data/file.txt --output /data/out.fasta
ENTRYPOINT ["adn-web"]
