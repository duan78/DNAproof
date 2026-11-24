# Guide de production

Ce guide décrit l'installation et l'exploitation du serveur web `adn-web`. La ligne de commande
`adn` n'a pas besoin de serveur : voir le [README](README.md#ligne-de-commande).

## 1. Ce que fait le serveur

`adn-web` reçoit un fichier, l'archive en séquences d'ADN, et rend l'archive FASTA à
télécharger. Il fait aussi l'inverse : il reçoit des lectures FASTA et rend le fichier d'origine,
à condition que l'empreinte SHA-256 embarquée dans l'archive corresponde.

- **Il est prévu pour une personne ou une petite équipe de confiance.** Il n'y a ni compte, ni
  authentification : quiconque peut joindre le serveur peut l'utiliser et télécharger les
  résultats des autres. Ne l'exposez pas sur Internet sans lui adjoindre une authentification.
- **Par défaut, il n'écoute que sur la machine elle-même**, et refuse les requêtes qui visent un
  autre nom d'hôte.
- **Chaque résultat est conservé 24 heures** par défaut, puis effacé.

## 2. Prérequis

- Rust 1.94 ou plus récent, pour construire ([rustup](https://rustup.rs)) ; ou Docker, pour
  utiliser l'image.
- Rien d'autre : la base SQLite est compilée dans l'exécutable, et le serveur ne parle pas TLS.

## 3. Construire

```sh
git clone https://github.com/duan78/DNAproof
cd DNAproof
cargo build --release --locked -p adn-cli -p adn-web
```

Deux exécutables sont produits dans `target/release/` : `adn` et `adn-web`. Le serveur a besoin
de ses pages à côté de lui : `crates/web/templates/` et `crates/web/static/`.

## 4. Lancer

Le plus simple, depuis le dossier du serveur :

```sh
cd crates/web
../../target/release/adn-web
```

```text
Serveur ADN 0.2.0 sur http://127.0.0.1:8080 (fichiers jusqu'à 25 Mio, 2 tâche(s) à la fois)
```

Le serveur lit `config.toml` dans le dossier courant, ou le chemin donné en argument :

```sh
adn-web /etc/adn/config.toml
```

Sans fichier, les valeurs par défaut s'appliquent. Un fichier illisible, ou qui contient une
clé inconnue, arrête le démarrage avec un message : un réglage n'est jamais ignoré en silence.

Un résultat n'est jamais laissé à moitié écrit : il l'est dans un fichier temporaire, renommé une
fois complet. Un serveur arrêté en pleine tâche ne laisse donc rien de trompeur derrière lui.

## 5. Configuration

Le fichier `config.toml` livré à la racine du dépôt est documenté ligne à ligne. Récapitulatif :

| Clé | Défaut | Rôle |
|-----|--------|------|
| `server.host` | `127.0.0.1` | adresse écoutée |
| `server.allowed_hosts` | `[]` | noms acceptés en plus de la boucle locale |
| `server.port` | `8080` | port écouté |
| `server.workers` | `2` | threads qui servent les requêtes |
| `server.upload_limit` | `26214400` | taille maximale des fichiers acceptés, en octets (25 Mio) |
| `server.max_concurrent_jobs` | `2` | encodages et décodages menés de front |
| `server.job_retention_minutes` | `1440` | conservation des résultats ; `0` pour ne jamais les effacer |
| `server.storage_dir` | `./uploads` | dossier des résultats |
| `server.static_files` | `./static` | feuille de style et scripts |
| `server.templates` | `./templates` | pages |
| `database.enabled` | `false` | conserver aussi les oligos en base |
| `database.type` | `sqlite` | `sqlite` ou `postgres` |
| `database.url` | `adn_storage.db` | fichier SQLite, ou URL `postgres://` |
| `database.max_connections` | `5` | connexions ouvertes |
| `logging.level` | `info` | `trace`, `debug`, `info`, `warn` ou `error` |
| `logging.format` | `compact` | `compact` ou `json` |

**Variables d'environnement.** Chaque clé se surcharge par `ADN_<SECTION>__<CLÉ>` (double tiret
souligné entre la section et la clé) :

```sh
ADN_SERVER__PORT=9000 adn-web
ADN_DATABASE__ENABLED=true ADN_DATABASE__URL=/var/lib/adn/base.db adn-web
```

`RUST_LOG` l'emporte sur `logging.level`, selon la syntaxe d'`env_logger` :

```sh
RUST_LOG=info,adn_web=debug adn-web
```

**Taille des fichiers.** Le fichier est lu en mémoire, et un fichier incompressible donne environ
huit fois sa taille d'ADN. Comptez, pour une tâche, environ dix fois la taille du fichier en
mémoire, et autant sur le disque pour l'archive. Avec `max_concurrent_jobs = 2` et la limite de
25 Mio, le serveur peut donc occuper jusqu'à 500 Mo de mémoire vive.

## 6. Déploiement

### Exécution directe

```sh
adn-web /etc/adn/config.toml
```

### systemd

```ini
# /etc/systemd/system/adn-web.service
[Unit]
Description=Serveur web d'archivage de fichiers en ADN
After=network.target

[Service]
User=adn
Group=adn
WorkingDirectory=/opt/adn
ExecStart=/opt/adn/adn-web /etc/adn/config.toml
Restart=on-failure

# Le serveur n'écrit que dans son dossier de résultats.
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/adn/results

[Install]
WantedBy=multi-user.target
```

Installation :

```sh
sudo useradd --system --home-dir /var/lib/adn --shell /usr/sbin/nologin adn
sudo mkdir -p /opt/adn /var/lib/adn/results /etc/adn
sudo cp target/release/adn target/release/adn-web /opt/adn/
sudo cp -r crates/web/templates crates/web/static /opt/adn/
sudo cp config.toml /etc/adn/config.toml
```

Le fichier `/etc/adn/config.toml` doit alors nommer des chemins absolus :

```toml
[server]
storage_dir = "/var/lib/adn/results"
static_files = "/opt/adn/static"
templates = "/opt/adn/templates"
```

Puis `sudo systemctl daemon-reload && sudo systemctl enable --now adn-web`.

### Docker

```sh
docker build -t adn .
docker run --rm -p 8080:8080 -v adn-results:/app/uploads adn
```

L'image contient aussi la ligne de commande :

```sh
docker run --rm --entrypoint adn -v "$PWD:/data" adn encode --input /data/fichier.txt --output /data/fichier.fasta
```

L'image est construite par le CI à chaque demande d'ajout. Elle n'est publiée sur aucun registre.

### Derrière un proxy inverse

Le serveur ne parle pas TLS : c'est le rôle du proxy. Un exemple avec nginx :

```nginx
server {
    listen 443 ssl;
    server_name adn.example.org;

    # La limite du serveur (25 Mio) et l'enveloppe du formulaire.
    client_max_body_size 26m;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_read_timeout 300s;
    }
}
```

Le serveur reste lié à `127.0.0.1` : seul le proxy l'atteint. Comme il préserve l'en-tête
`Host`, déclarez le nom public du proxy, sans quoi chaque requête est refusée :

```toml
[server]
host = "127.0.0.1"
allowed_hosts = ["adn.example.org"]
```

Ajoutez à ce niveau l'authentification dont vous avez besoin (`auth_basic`, ou un fournisseur
d'identité devant nginx) : le serveur n'en apporte aucune.

## 7. Base de données

La base est facultative. Elle garde, pour chaque archive encodée par le serveur, les oligos et de
quoi les retrouver : nom du fichier d'origine, empreinte, schéma, mesures.

**À quoi elle sert.** Sans elle, les résultats ne vivent que dans `storage_dir` et dans la mémoire
du serveur : un redémarrage les fait disparaître de la liste des tâches. Avec elle, un serveur qui
redémarre retrouve ses archives ; un fichier de résultat disparu est réécrit depuis les oligos
rangés en base.

**Ce qu'elle ne fait pas.** Elle ne conserve pas les fichiers décodés : l'archive FASTA suffit à
les restituer. Elle ne remplace pas une sauvegarde de `storage_dir`.

Les migrations sont appliquées au démarrage. Une base inaccessible arrête le serveur : le
démarrage ne continue jamais avec une base silencieusement désactivée.

### SQLite

```toml
[database]
enabled = true
type = "sqlite"
url = "/var/lib/adn/base.db"
```

Le fichier doit être dans un dossier accessible en écriture au serveur ; SQLite y crée aussi deux
fichiers annexes (`-wal` et `-shm`).

### PostgreSQL

```toml
[database]
enabled = true
type = "postgres"
url = "postgres://adn:mot_de_passe@db.example.org:5432/adn"
```

L'utilisateur doit pouvoir créer des tables. Le mot de passe passe par l'environnement plutôt que
par le fichier, si le fichier est lisible d'autres personnes :

```sh
ADN_DATABASE__URL='postgres://adn:mot_de_passe@db.example.org:5432/adn' adn-web
```

## 8. Sécurité

Ce que fait le serveur, et ce qu'il ne fait pas.

- **Boucle locale par défaut.** Lié à `127.0.0.1`, il refuse toute requête dont l'en-tête `Host`
  ne désigne pas la machine elle-même (`127.0.0.1`, `localhost`, `::1`). Un site malveillant ne
  peut pas, en faisant rebinder son domaine vers `127.0.0.1`, lire les résultats comme s'il était
  la page d'origine.
- **Pas de CORS.** Seules les pages du serveur lisent son API.
- **Taille et nombre bornés.** Un fichier dépasse la limite : requête refusée (413). Plus de
  8 × `max_concurrent_jobs` tâches en attente : requête refusée (429).
- **Fichiers restitués en pièces jointes**, avec `X-Content-Type-Options: nosniff` : un fichier
  décodé, d'origine quelconque, n'est jamais interprété comme une page du site.
- **Noms nettoyés.** Le nom d'un fichier envoyé perd son chemin et ses caractères de contrôle
  avant de servir à un téléchargement. L'identifiant d'une tâche doit être un UUID : il n'y a pas
  de chemin à traverser.
- **Pas d'authentification, pas de chiffrement.** À ajouter devant le serveur pour tout autre
  usage qu'un poste de travail isolé.
- **Les téléchargements ne s'expirent pas individuellement.** Quiconque connaît l'identifiant
  d'une tâche la télécharge jusqu'à sa suppression.

## 9. Sauvegardes et reprise

Une archive FASTA est un fichier comme un autre : sauvegardez `storage_dir` avec vos autres
données, ou téléchargez les archives qui comptent. L'archive se suffit à elle-même : elle se
décode sans le serveur, avec la ligne de commande :

```sh
adn decode --input archive.fasta --output fichier
```

La base de données est un miroir : sauvegardez-la avec l'outil de votre base
(`sqlite3 base.db .dump`, `pg_dump`), ou laissez-la se reconstruire, puisqu'elle ne contient que
ce que le serveur a produit.

## 10. Supervision

- `GET /health` répond `200` avec l'état de la base de données, ou `503` si elle est activée et ne
  répond plus. C'est le contrôle fait par l'image Docker toutes les 30 secondes.
- `GET /api/stats` rend le nombre de tâches par état, les limites en vigueur et le bilan de la
  base.
- Les requêtes sont journalisées sur la sortie d'erreur ; `/health` n'y figure pas, pour ne pas
  noyer le journal. `logging.format = "json"` donne des lignes lisibles par une machine.

## 11. Dépannage

| Message ou symptôme | Cause et remède |
|---------------------|-----------------|
| `écoute impossible sur 127.0.0.1:8080` | le port est pris : changez `server.port` |
| `dossier des fichiers statiques introuvable` | lancez depuis `crates/web`, ou fixez `static_files` et `templates` |
| `configuration inapplicable … unknown field` | une clé est mal orthographiée dans `config.toml` ou dans l'environnement |
| `base de données indisponible` | la base est activée mais inaccessible ; vérifiez `database.url`, ou désactivez-la |
| `403 hôte refusé` | la requête arrive avec un `Host` inattendu : ajoutez-le à `server.allowed_hosts` |
| `413` à l'envoi | le fichier dépasse `server.upload_limit` ; pensez aussi à `client_max_body_size` du proxy |
| `429 trop de tâches en attente` | la file d'attente est pleine : réessayez, ou augmentez `max_concurrent_jobs` |
| `500` à la suppression | les fichiers sont effacés, mais pas leur archive en base : voir le journal |
| `archive au format de la version 0.1` | l'archive a été produite par l'ancienne version, qui ne se lit plus (voir le [journal des changements](CHANGELOG.md)) |
| `aucun manifeste lisible` | le fichier n'est pas une archive de ce format, ou ses erreurs sont trop nombreuses |
| `le corps reconstitué est incohérent` | trop de lectures abîmées : voir [Limites](#12-limites-connues) |

## 12. Limites connues

- **Fichiers de plus de 25 Mio** : relevez `server.upload_limit`, en gardant en mémoire le coût
  (environ dix fois la taille du fichier).
- **Petits fichiers** : DNA Fountain produit beaucoup plus d'oligos que nécessaire en dessous de
  quelques dizaines de kilooctets — environ 2,9 gouttes par bloc pour 4 Ko. C'est le prix d'une
  archive qui reste décodable après des pertes.
- **Erreurs de lecture** : ce que chaque schéma tolère est mesuré dans
  [les schémas d'encodage](docs/encoding_schemes.md#tolérance-aux-erreurs). En bref, DNA Fountain
  ne corrige qu'une erreur par oligo avec sa parité par défaut, et `strands` ne traverse ni les
  insertions et suppressions, ni plus de 5 % de perte sur un gros fichier. Seul `hedges` corrige
  les insertions et les suppressions : 4,5 % d'erreurs des trois sortes réunies par base à son
  rendement ½, 15 % à ¼. Le serveur web règle la marge de perte et le rendement de `hedges`,
  mais pas la parité :
  pour un canal difficile, passez par la ligne de commande.
