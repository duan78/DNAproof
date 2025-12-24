# 🧬 ADN Data Storage - Guide de Production

## 🚀 Déploiement en Production

### Prérequis

- Rust 1.90+ (recommandé: dernière version stable)
- SQLite 3.35+ ou PostgreSQL 12+
- 4GB+ RAM (8GB+ recommandé pour les gros fichiers)
- 10GB+ espace disque

### Installation

```bash
# Cloner le dépôt
git clone https://github.com/duan78/DNAproof.git
cd DNAproof

# Construire en mode release
cargo build --release

# Créer le fichier de configuration
cp config.toml.example config.toml

# Modifier la configuration selon vos besoins
# config.toml
```

### Configuration

#### Configuration de base (`config.toml`)

```toml
[server]
host = "0.0.0.0"  # Écouter sur toutes les interfaces
host = "127.0.0.1"  # Écouter uniquement en local
port = 8080
workers = 8  # Nombre de workers (recommandé: nombre de cœurs CPU)
upload_limit = 104857600  # 100MB (ajuster selon vos besoins)
static_files = "./static"
templates = "./templates"

[database]
enabled = true
url = "adn_storage.db"  # SQLite
# url = "postgres://user:password@localhost/adn_storage"  # PostgreSQL
max_connections = 10

[logging]
level = "info"  # trace, debug, info, warn, error
format = "compact"  # compact ou json
```

### Déploiement

#### Option 1: Exécution directe

```bash
# Démarrer le serveur
./target/release/adn-web

# Avec logging avancé
RUST_LOG=info ./target/release/adn-web
```

#### Option 2: Avec systemd (Linux)

Créer un fichier `/etc/systemd/system/adn-storage.service`:

```ini
[Unit]
Description=ADN Data Storage Server
After=network.target

[Service]
User=adn
Group=adn
WorkingDirectory=/opt/adn-storage
ExecStart=/opt/adn-storage/target/release/adn-web
Restart=always
RestartSec=5
Environment="RUST_LOG=info"

[Install]
WantedBy=multi-user.target
```

Puis:
```bash
sudo systemctl daemon-reload
sudo systemctl enable adn-storage
sudo systemctl start adn-storage
sudo systemctl status adn-storage
```

#### Option 3: Avec Docker

Créer un `Dockerfile`:

```dockerfile
FROM rust:1.90 as builder

WORKDIR /usr/src/adn-storage
COPY . .

RUN cargo build --release

FROM debian:bullseye-slim

RUN apt-get update && apt-get install -y \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/adn-storage/target/release/adn-web /usr/local/bin/
COPY --from=builder /usr/src/adn-storage/config.toml /etc/adn-storage/
COPY --from=builder /usr/src/adn-storage/static /usr/local/share/adn-storage/static
COPY --from=builder /usr/src/adn-storage/templates /usr/local/share/adn-storage/templates

WORKDIR /usr/local/share/adn-storage

EXPOSE 8080

CMD ["adn-web"]
```

Construire et exécuter:
```bash
docker build -t adn-storage .
docker run -p 8080:8080 -v ./data:/data adn-storage
```

### Configuration Avancée

#### Optimisation des performances

Pour les serveurs avec beaucoup de RAM:

```toml
[server]
workers = 16  # Pour les serveurs multi-cœurs

[database]
max_connections = 20
```

#### Configuration pour les gros fichiers

```toml
[server]
upload_limit = 1073741824  # 1GB

# Dans le code, ajuster aussi:
# - chunk_size dans EncoderConfig
# - redundancy pour les gros fichiers
```

### Monitoring

#### Métriques

Le serveur expose un endpoint de santé:
```bash
curl http://localhost:8080/health
```

#### Logging

Les logs sont disponibles en JSON pour l'intégration avec les outils de monitoring:

```toml
[logging]
level = "info"
format = "json"
```

Exemple de log:
```json
{"timestamp":"2023-11-15T10:30:00Z","level":"INFO","message":"Nouvelle requête d'encodage","target":"adn_web::routes","span":{"name":"api_encode"}}
```

### Sécurité

#### Recommandations

1. **HTTPS**: Toujours utiliser HTTPS en production avec un certificat valide
2. **Authentification**: Ajouter une authentification pour les endpoints sensibles
3. **Rate Limiting**: Configurer un rate limiting pour éviter les abus
4. **Mises à jour**: Garder le serveur et les dépendances à jour

#### Configuration HTTPS avec Nginx

```nginx
server {
    listen 443 ssl;
    server_name adn-storage.example.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /static/ {
        alias /usr/local/share/adn-storage/static/;
        expires 30d;
    }
}
```

### Maintenance

#### Sauvegardes

Pour SQLite:
```bash
# Sauvegarde de la base de données
sqlite3 adn_storage.db ".backup 'backup.db'"

# Restauration
sqlite3 adn_storage.db ".restore 'backup.db'"
```

Pour PostgreSQL:
```bash
# Sauvegarde
pg_dump -U user -h localhost adn_storage > backup.sql

# Restauration
psql -U user -h localhost -d adn_storage < backup.sql
```

#### Mises à jour

1. Arrêter le service
2. Sauvegarder la base de données
3. Mettre à jour le code
4. Construire la nouvelle version
5. Redémarrer le service

### Résolution des problèmes

#### Problèmes courants

**Problème**: Le serveur ne démarre pas
- Vérifier les permissions sur les fichiers
- Vérifier que les ports ne sont pas déjà utilisés
- Consulter les logs pour les erreurs

**Problème**: Performances lentes
- Vérifier la configuration de la base de données
- Augmenter le nombre de workers
- Vérifier l'utilisation CPU/RAM

**Problème**: Erreurs de connexion à la base de données
- Vérifier l'URL de connexion
- Vérifier que le serveur de base de données est en cours d'exécution
- Vérifier les permissions

### Support

Pour obtenir de l'aide:
- Consulter la documentation technique
- Ouvrir une issue sur GitHub
- Contacter l'équipe de support

## 📊 Métriques de Performance

### Capacité

- **Petits fichiers** (<1MB): ~100 fichiers/minute
- **Fichiers moyens** (1-10MB): ~50 fichiers/minute
- **Gros fichiers** (10-100MB): ~10 fichiers/minute

### Latence

- Temps d'encodage: ~100ms/MB
- Temps de décodage: ~50ms/MB
- Latence API: <50ms (95th percentile)

### Ressources

- **CPU**: 1-4 cœurs recommandés
- **RAM**: 4GB minimum, 8GB+ pour les gros fichiers
- **Disque**: 10GB+ pour le stockage des séquences

## 🔧 Configuration Optimale

### Pour un serveur dédié

```toml
[server]
workers = 8  # 8 cœurs CPU
upload_limit = 1073741824  # 1GB

[database]
enabled = true
url = "postgres://adn_user:secure_password@localhost/adn_storage"
max_connections = 20

[logging]
level = "info"
format = "json"
```

### Pour un environnement de développement

```toml
[server]
workers = 2
upload_limit = 10485760  # 10MB

[database]
enabled = true
url = "adn_storage_dev.db"
max_connections = 5

[logging]
level = "debug"
format = "compact"
```

## 📝 Journal des Changements

### Version 1.0.0

- Première version stable
- Support complet de l'encodage/décodage
- Interface web et API REST
- Base de données SQLite/PostgreSQL
- Benchmarks et tests d'intégration

### Version 1.1.0 (Planifiée)

- Authentification et autorisation
- Interface utilisateur améliorée
- Support pour les fichiers très volumineux (>1GB)
- Optimisations supplémentaires

## 🤝 Contribution

Les contributions sont les bienvenues! Veuillez consulter le fichier CONTRIBUTING.md pour les directives.

## 📄 Licence

Ce projet est sous licence MIT OR Apache-2.0. Voir le fichier LICENSE pour plus de détails.

## 🔗 Liens

- **GitHub**: https://github.com/duan78/DNAproof
- **Documentation**: https://docs.adn-storage.com
- **Support**: support@adn-storage.com

---

🧬 ADN Data Storage - Stockage de données dans l'ADN virtuel | Version 1.0.0