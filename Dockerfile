# syntax=docker/dockerfile:1
# Serveur web et ligne de commande d'archivage de fichiers en ADN.
#
#     docker build -t adn .
#     docker run --rm -p 8080:8080 -v adn-uploads:/app/uploads adn
#     docker run --rm --entrypoint adn adn encode --input /data/fichier.txt --output /data/fichier.fasta

# ─── Builder ─────────────────────────────────────────────────────────────────
# La version minimale prise en charge par le projet (`rust-version` du workspace) : une image
# qui construit prouve que cette version suffit.
FROM rust:1.94-slim AS builder

WORKDIR /build

# Manifestes et verrou d'abord : la couche des dépendances reste en cache si seules les sources
# changent. Le verrou est suivi dans le dépôt, et la construction s'y tient.
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

RUN cargo build --release --locked -p adn-cli -p adn-web

# ─── Runtime ─────────────────────────────────────────────────────────────────
# Ni OpenSSL ni base : SQLite est compilé dans l'exécutable, et le serveur ne parle pas TLS
# (c'est le rôle d'un proxy).
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# Utilisateur sans privilèges.
RUN useradd --system --create-home --shell /usr/sbin/nologin adn

WORKDIR /app

COPY --from=builder /build/target/release/adn /build/target/release/adn-web /usr/local/bin/

# Pages, fichiers statiques et configuration.
COPY crates/web/templates/ ./templates/
COPY crates/web/static/ ./static/
COPY config.toml ./

# Dossier des résultats, accessible à l'utilisateur du serveur.
RUN mkdir -p /app/uploads && chown -R adn:adn /app
USER adn

# `config.toml` n'écoute que sur la boucle locale, ce qui protège un serveur lancé sur son
# poste ; dans un conteneur, il faut écouter sur toutes les interfaces pour être joignable.
ENV ADN_SERVER__HOST=0.0.0.0

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["adn-web"]
