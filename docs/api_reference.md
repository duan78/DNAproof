# Référence de l'API web

L'API du serveur `adn-web` encode et décode des fichiers par tâches : une requête dépose le
fichier et rend un identifiant ; on suit la tâche jusqu'à son terme ; on télécharge le résultat.
Les pages du serveur utilisent cette API, et rien d'autre.

- **URL de base** : `http://127.0.0.1:8080` par défaut.
- **Format** : JSON partout, sauf les fichiers téléchargés.
- **Origine** : l'API ne se lit que depuis les pages du serveur. Il n'y a pas de CORS : un script
  d'un autre site ne reçoit jamais les réponses.
- **Hôtes** : lié à la boucle locale, le serveur refuse les requêtes dont l'en-tête `Host` n'est pas
  la machine elle-même (`403`). Les hôtes acceptés se règlent dans `config.toml`
  ([guide de production](../PRODUCTION_GUIDE.md#derrière-un-proxy-inverse)).

## Pages

| Chemin | Rôle |
|--------|------|
| `GET /` | accueil, liste des tâches récentes |
| `GET /encode` | encoder un fichier |
| `GET /decode` | décoder une archive |
| `GET /static/…` | feuille de style et scripts des pages |

## Tâches

Une tâche est un encodage ou un décodage mené en arrière-plan. Son état :

```json
{
  "id": "2e0863a7-ce24-4bc8-b0bb-35583dce05af",
  "kind": "encode",
  "status": "complete",
  "source_name": "fichier-essai.txt",
  "file_name": "fichier-essai.txt.fasta",
  "download_url": "/download/fasta/2e0863a7-ce24-4bc8-b0bb-35583dce05af",
  "sha256": "701a70de66b63a5681e6fb85bd153a5e5233f019d123be0d3df176674072237d",
  "size": 9416,
  "stats": { "…": "…" },
  "created_at": "2026-11-23T16:42:42.6807297Z",
  "updated_at": "2026-11-23T16:42:42.6932962Z"
}
```

| Champ | Présent | Contenu |
|-------|---------|---------|
| `id` | toujours | un UUID |
| `kind` | toujours | `encode` ou `decode` |
| `status` | toujours | `queued`, `running`, `complete` ou `failed` |
| `stage` | en attente, en cours | l'étape en cours (`encodage`, `décodage`, `écriture du fichier`…) |
| `source_name` | toujours | nom du fichier envoyé, sans chemin |
| `file_name` | terminée | nom proposé au téléchargement |
| `download_url` | réussie | chemin du résultat |
| `sha256` | réussie | empreinte du fichier : celui d'origine à l'encodage, celui restitué au décodage |
| `size` | réussie | taille du résultat, en octets |
| `stats` | réussie | mesures propres au schéma (voir plus bas) |
| `error` | échouée | pourquoi |
| `created_at`, `updated_at` | toujours | dates au format RFC 3339 |

## Endpoints

### `POST /api/encode`

Encode un fichier. Requête `multipart/form-data` :

| Champ | Requis | Contenu |
|-------|--------|---------|
| `file` | oui | le fichier à archiver ; un seul par requête |
| `scheme` | non | `fountain` (défaut), `strands`, `goldman`, `grass` ou `ldpc` |
| `compression` | non | `auto` (défaut), `none`, `lz4` ou `zstd` |
| `redundancy` | non | marge de perte de DNA Fountain : un nombre de 1 à 100, `1.1` par défaut |

Tout autre champ est refusé : un réglage ignoré en silence est une archive différente de celle
qu'on croyait.

**Réponse `202`** :

```json
{"job_id": "2e0863a7-ce24-4bc8-b0bb-35583dce05af", "status": "queued", "location": "/api/jobs/2e0863a7-ce24-4bc8-b0bb-35583dce05af"}
```

Le nom proposé au téléchargement est celui du fichier d'origine, suivi de `.fasta` (ou de
`.adn.fasta` si le fichier d'origine est déjà une archive).

### `POST /api/decode`

Décode une archive. Requête `multipart/form-data` :

| Champ | Requis | Contenu |
|-------|--------|---------|
| `file` | oui | les lectures, en FASTA ou en FASTQ ; plusieurs lectures d'une même archive peuvent être copiées dans le fichier |
| `expected_sha256` | non | empreinte que doit avoir le fichier restitué (64 chiffres hexadécimaux) |

Le schéma et la compression sont lus dans l'archive : il n'y a rien à indiquer.

**Réponse `202`** : même forme qu'à l'encodage.

Le nom proposé est celui de l'archive sans son suffixe (`.fasta`, `.fas`, `.fa`). Avec une empreinte
attendue, un fichier d'une autre empreinte n'est jamais rendu : la tâche échoue.

### `GET /api/jobs/{id}`

L'état d'une tâche. `404` si elle est inconnue ou retirée, `400` si l'identifiant n'est pas un
UUID.

### `GET /api/jobs`

Les tâches conservées, de la plus récente à la plus ancienne :

```json
{"jobs": [ { "…": "…" } ]}
```

### `DELETE /api/jobs/{id}`

Retire une tâche terminée, efface ses fichiers et, si la base de données est activée, son
archive. `204` sans corps.

Une tâche en attente ou en cours n'est pas annulable : la requête est refusée (`409`) et la tâche
continue. La retirer une fois terminée.

### `GET /download/fasta/{id}`

L'archive FASTA d'un encodage réussi. `text/plain`, en pièce jointe, avec le nom du fichier
dans `Content-Disposition` :

```http
Content-Disposition: attachment; filename="essai.txt.fasta"; filename*=UTF-8''essai.txt.fasta
```

### `GET /download/{id}`

Le fichier restitué par un décodage réussi. `application/octet-stream`, en pièce jointe.

Un téléchargement ne rend un fichier que pour une tâche *réussie* du bon type : `404` sinon.

### `GET /api/stats`

L'état du serveur :

```json
{
  "version": "0.2.0",
  "started_at": "2026-11-23T15:04:46.1722536Z",
  "uptime_seconds": 33.4869678,
  "limits": {"upload_bytes": 26214400, "concurrent_jobs": 2, "retention_minutes": 1440},
  "schemes": ["fountain", "strands", "goldman", "grass", "ldpc"],
  "jobs": {"queued": 0, "running": 0, "complete": 2, "failed": 0},
  "database": {"enabled": true, "archives": 1, "oligos": 127}
}
```

`database` vaut `{"enabled": false}` si la base est désactivée, et porte une clé `error` si elle
ne répond plus.

### `GET /health`

```json
{"status": "ok", "version": "0.2.0", "database": {"enabled": true, "state": "ok"}}
```

`200` si le serveur répond, `503` si la base est activée et ne répond plus. C'est le contrôle
utilisé par l'image Docker.

## Erreurs

Toujours en JSON, avec le statut répété dans le corps :

```json
{"code": 400, "error": "identifiant de tâche invalide"}
```

| Statut | Quand |
|--------|-------|
| `400` | aucun fichier, deux fichiers, champ inconnu, valeur inconnue (`scheme`, `compression`, `redundancy`, `expected_sha256`), identifiant mal formé |
| `403` | l'hôte de la requête n'est pas accepté |
| `404` | tâche inconnue, rien à télécharger, fichier disparu |
| `409` | retrait d'une tâche en attente ou en cours |
| `413` | le fichier dépasse la limite configurée |
| `429` | plus de 8 × `max_concurrent_jobs` tâches attendent déjà |
| `500` | résultat effacé du disque, mais pas de la base de données |

Les erreurs d'encodage et de décodage ne sont pas des erreurs HTTP : la requête est acceptée
(`202`), puis la tâche échoue avec un `error` explicite — archive au format de la version 0.1,
aucun manifeste lisible, empreinte inattendue, corps incohérent…

## Mesures

`stats` d'un **encodage** :

| Champ | Contenu |
|-------|---------|
| `scheme`, `compression` | tels qu'ils ont été appliqués |
| `original_bytes`, `body_bytes` | taille du fichier d'origine, puis de l'enveloppe (compressée ou non) |
| `units` | blocs (DNA Fountain), fenêtres (`goldman`) ou brins de données (`strands`, `grass`, `ldpc`) |
| `oligos`, `data_oligos`, `manifest_copies` | nombre d'oligos, au total, de données et de manifestes |
| `oligo_length_nt` | longueur d'un oligo |
| `nucleotides` | total |
| `bits_per_nucleotide` | densité : les bits du fichier d'origine par nucléotide |
| `redundancy` | marge de perte (DNA Fountain) |
| `droplets_per_unit` | gouttes par bloc (DNA Fountain) |
| `candidates_tried` | candidats examinés avant d'obtenir les oligos conformes |
| `encode_seconds` | durée |
| `archive_bytes` | taille de l'archive FASTA |
| `stored_in_database` | l'archive a-t-elle été rangée en base |

`stats` d'un **décodage** :

| Champ | Contenu |
|-------|---------|
| `scheme`, `compression` | lus dans l'archive |
| `reads_total` | lectures reçues |
| `reads_invalid`, `reads_bad_length` | écartées : caractères interdits, mauvaise longueur |
| `manifest_candidates`, `manifest_agreeing` | lectures ressemblant à un manifeste, et manifestes identiques retenus |
| `corrected`, `uncorrectable` | lectures corrigées par la parité, lectures irrécupérables |
| `units_used` | gouttes ou brins utilisés |
| `contested` | lectures en désaccord sur un même brin (`strands`, `grass`), ou bases du message mises à égalité (`goldman`) |
| `blocks_repaired`, `strands_repaired` | blocs et brins rétablis par le code externe (`strands`, `grass`) |
| `retried_without_corrected_reads` | le second passage, sans les lectures corrigées, a été nécessaire (DNA Fountain) |
| `file_bytes`, `decode_seconds` | taille du fichier, durée |

## Exemples

Avec `curl` :

```sh
# Encoder, puis suivre la tâche.
curl -F "file=@rapport.pdf" -F "scheme=fountain" -F "redundancy=1.5" http://127.0.0.1:8080/api/encode
curl http://127.0.0.1:8080/api/jobs/<job_id>
curl -OJ http://127.0.0.1:8080/download/fasta/<job_id>

# Décoder avec une empreinte attendue.
curl -F "file=@rapport.pdf.fasta" \
     -F "expected_sha256=<64 chiffres hexadécimaux>" \
     http://127.0.0.1:8080/api/decode
```

En JavaScript, depuis la page d'un outil :

```js
async function encoder(fichier) {
  const formulaire = new FormData();
  formulaire.append("file", fichier);
  formulaire.append("scheme", "strands");
  const dépôt = await fetch("/api/encode", { method: "POST", body: formulaire });
  if (!dépôt.ok) throw new Error((await dépôt.json()).error);
  const { job_id } = await dépôt.json();

  for (;;) {
    const réponse = await fetch(`/api/jobs/${job_id}`);
    const tâche = await réponse.json();
    if (tâche.status === "complete") return tâche;
    if (tâche.status === "failed") throw new Error(tâche.error);
    await new Promise((résous) => setTimeout(résous, 400));
  }
}
```

En Python :

```python
import time
import requests

serveur = "http://127.0.0.1:8080"


def encoder(chemin, schéma="fountain"):
    with open(chemin, "rb") as fichier:
        dépôt = requests.post(
            f"{serveur}/api/encode",
            files={"file": fichier},
            data={"scheme": schéma},
        )
    dépôt.raise_for_status()
    tâche = dépôt.json()["job_id"]
    while True:
        état = requests.get(f"{serveur}/api/jobs/{tâche}").json()
        if état["status"] == "complete":
            return état
        if état["status"] == "failed":
            raise RuntimeError(état["error"])
        time.sleep(0.4)


archive = encoder("rapport.pdf")
contenu = requests.get(serveur + archive["download_url"]).content
requests.delete(f"{serveur}/api/jobs/{archive['id']}")
```

## Limites

- **Taille** : 25 Mio par fichier par défaut (`server.upload_limit`).
- **File d'attente** : `429` au-delà de 8 × `max_concurrent_jobs` tâches en attente.
- **Conservation** : les résultats sont effacés après `server.job_retention_minutes` (24 heures par
  défaut), avec leur tâche.
- **Résultats en mémoire** : un résultat n'existe que sur son serveur. Sans base de données, un
  redémarrage les retire de la liste ; téléchargez ce qui compte.
