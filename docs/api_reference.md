# REST API Reference

Documentation de l'API REST pour le système de stockage ADN.

## Base URL

```
http://localhost:8080
```

---

## Endpoints

### Encodage

#### POST /api/encode

Encode un fichier en séquences ADN.

**Requête**:
```http
POST /api/encode HTTP/1.1
Content-Type: multipart/form-data

file: <fichier>
algorithm: erlich_zielinski_2017 (optionnel, défaut: fountain)
redundancy: 1.5 (optionnel, défaut: 1.5)
compression: true (optionnel, défaut: true)
```

**Paramètres**:

| Paramètre | Type | Requis | Description |
|-----------|------|--------|-------------|
| `file` | file | Oui | Fichier à encoder |
| `algorithm` | string | Non | Schéma d'encodage (`fountain`, `goldman`, `goldman2013`, `grass2015`) |
| `redundancy` | float | Non | Facteur de redondance (1.0-3.0, défaut: 1.5) |
| `compression` | boolean | Non | Activer compression (défaut: true) |

**Réponse**: 202 Accepted
```json
{
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "processing",
  "message": "Encodage en cours"
}
```

**Erreurs**:
- `400 Bad Request`: Paramètres invalides
- `500 Internal Server Error`: Erreur serveur

**Exemple curl**:
```bash
curl -X POST http://localhost:8080/api/encode \
  -F "file=@document.txt" \
  -F "algorithm=fountain" \
  -F "redundancy=1.5"
```

---

### Statut de Job

#### GET /api/jobs/{job_id}

Récupère le statut d'un job d'encodage/décodage.

**Requête**:
```http
GET /api/jobs/{job_id} HTTP/1.1
```

**Paramètres**:

| Paramètre | Type | Description |
|-----------|------|-------------|
| `job_id` | string | UUID du job |

**Réponse**: 200 OK
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "complete",
  "progress": 1.0,
  "created_at": "2025-12-25T10:30:00Z",
  "updated_at": "2025-12-25T10:30:05Z",
  "result": {
    "download_url": "/download/fasta/550e8400-e29b-41d4-a716-446655440000",
    "stats": {
      "sequence_count": 1523,
      "avg_length": 150.0,
      "gc_ratio": 0.5,
      "bits_per_base": 1.92,
      "total_bases": 228450
    }
  },
  "error": null
}
```

**Statuts possibles**:
- `pending`: En attente
- `processing`: En cours de traitement
- `complete`: Terminé avec succès
- `failed`: Échoué

**Erreurs**:
- `404 Not Found`: Job non trouvé

**Exemple curl**:
```bash
curl http://localhost:8080/api/jobs/550e8400-e29b-41d4-a716-446655440000
```

---

### Téléchargement FASTA

#### GET /download/fasta/{job_id}

Télécharge le fichier FASTA généré.

**Requête**:
```http
GET /download/fasta/{job_id} HTTP/1.1
```

**Paramètres**:

| Paramètre | Type | Description |
|-----------|------|-------------|
| `job_id` | string | UUID du job |

**Réponse**: 200 OK
```
Content-Type: text/x-fasta
Content-Disposition: attachment; filename="{job_id}.fasta"

>sequence_0
ATCGATCGATCGATCG...
>sequence_1
GCTAGCTAGCTAGCTA...
...
```

**Erreurs**:
- `404 Not Found`: Fichier non trouvé ou job non terminé

**Exemple curl**:
```bash
curl -O http://localhost:8080/download/fasta/550e8400-e29b-41d4-a716-446655440000
```

---

### Liste des Jobs

#### GET /api/jobs

Liste tous les jobs (optionnellement filtrés par statut).

**Requête**:
```http
GET /api/jobs?status=complete&limit=10 HTTP/1.1
```

**Paramètres Query**:

| Paramètre | Type | Requis | Description |
|-----------|------|--------|-------------|
| `status` | string | Non | Filtrer par statut (`pending`, `processing`, `complete`, `failed`) |
| `limit` | integer | Non | Nombre max de résultats (défaut: 50) |

**Réponse**: 200 OK
```json
{
  "jobs": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "status": "complete",
      "created_at": "2025-12-25T10:30:00Z"
    },
    {
      "id": "660e8400-e29b-41d4-a716-446655440001",
      "status": "processing",
      "created_at": "2025-12-25T10:31:00Z"
    }
  ],
  "total": 2
}
```

---

### Statistiques Système

#### GET /api/stats

Retourne les statistiques du système.

**Requête**:
```http
GET /api/stats HTTP/1.1
```

**Réponse**: 200 OK
```json
{
  "total_jobs": 1523,
  "active_jobs": 3,
  "completed_jobs": 1500,
  "failed_jobs": 20,
  "total_sequences": 234567,
  "avg_processing_time": 5.2,
  "uptime": 86400
}
```

---

### Suppression de Job

#### DELETE /api/jobs/{job_id}

Supprime un job et ses fichiers associés.

**Requête**:
```http
DELETE /api/jobs/{job_id} HTTP/1.1
```

**Réponse**: 204 No Content (succès)

**Erreurs**:
- `404 Not Found`: Job non trouvé
- `403 Forbidden`: Job en cours de traitement

---

## Schémas d'Encodage Supportés

| Valeur | Algorithme | Description |
|--------|-----------|-------------|
| `fountain` | DNA Fountain (Custom) | Implémentation custom, utiliser `erlich_zielinski_2017` à la place |
| `erlich_zielinski_2017` | DNA Fountain EZ 2017 | **Recommandé** - Meilleure densité et tolérance aux erreurs |
| `goldman` | Goldman Simple | Legacy, utiliser `goldman2013` à la place |
| `goldman2013` | Goldman 2013 | Bon pour données texte/répétitives |
| `grass2015` | Grass 2015 | Haute fiabilité avec Reed-Solomon |

---

## Codes d'Erreur

| Code HTTP | Type d'Erreur | Description |
|-----------|---------------|-------------|
| `400` | `BadRequest` | Paramètres de requête invalides |
| `404` | `NotFound` | Ressource non trouvée |
| `413` | `PayloadTooLarge` | Fichier trop volumineux (max: 100MB) |
| `422` | `UnprocessableEntity` | Format de fichier invalide |
| `500` | `InternalServerError` | Erreur serveur interne |
| `503` | `ServiceUnavailable` | Service temporairement indisponible |

**Format d'erreur**:
```json
{
  "error": "Validation failed",
  "message": "File size exceeds maximum limit of 100MB",
  "code": 413
}
```

---

## Limitations

### Taille de Fichier

- Maximum: **100 MB** par fichier
- Recommandé: < 10 MB pour performance optimale

### Taux de Requêtes

- Pas de limite stricte (dépend des ressources serveur)
- Recommandé: < 10 requêtes/seconde

### Rétention de Fichiers

- Fichiers FASTA: 7 jours
- Métadonnées de jobs: 30 jours

---

## Exemples d'Utilisation

### JavaScript/Fetch

```javascript
// Encoder un fichier
async function encodeFile(file) {
  const formData = new FormData();
  formData.append('file', file);
  formData.append('algorithm', 'erlich_zielinski_2017');
  formData.append('redundancy', '1.5');

  const response = await fetch('/api/encode', {
    method: 'POST',
    body: formData
  });

  const result = await response.json();
  return result.job_id;
}

// Poller le statut
async function pollJobStatus(jobId) {
  while (true) {
    const response = await fetch(`/api/jobs/${jobId}`);
    const job = await response.json();

    if (job.status === 'complete') {
      return job.result;
    } else if (job.status === 'failed') {
      throw new Error(job.error);
    }

    // Attendre 2 secondes avant de réessayer
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
}

// Télécharger le FASTA
async function downloadFasta(jobId) {
  const response = await fetch(`/download/fasta/${jobId}`);
  const fasta = await response.text();

  // Créer un blob et télécharger
  const blob = new Blob([fasta], { type: 'text/plain' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `${jobId}.fasta`;
  a.click();
}

// Utilisation complète
const fileInput = document.getElementById('fileInput');
fileInput.addEventListener('change', async (e) => {
  const file = e.target.files[0];
  const jobId = await encodeFile(file);
  console.log('Job ID:', jobId);

  const result = await pollJobStatus(jobId);
  console.log('Stats:', result.stats);

  await downloadFasta(jobId);
  console.log('Téléchargement terminé!');
});
```

### Python/Requests

```python
import requests
import time

BASE_URL = "http://localhost:8080"

def encode_file(file_path, algorithm="erzielinski_2017", redundancy=1.5):
    """Encode un fichier en ADN."""
    with open(file_path, 'rb') as f:
        files = {'file': f}
        data = {
            'algorithm': algorithm,
            'redundancy': redundancy
        }
        response = requests.post(f"{BASE_URL}/api/encode", files=files, data=data)

    return response.json()['job_id']

def poll_job(job_id, max_wait=300):
    """Attend qu'un job se termine."""
    start_time = time.time()

    while True:
        response = requests.get(f"{BASE_URL}/api/jobs/{job_id}")
        job = response.json()

        if job['status'] == 'complete':
            return job['result']
        elif job['status'] == 'failed':
            raise Exception(job['error'])

        if time.time() - start_time > max_wait:
            raise TimeoutError("Job timeout")

        time.sleep(2)

def download_fasta(job_id, output_path):
    """Télécharge le fichier FASTA."""
    response = requests.get(f"{BASE_URL}/download/fasta/{job_id}")

    with open(output_path, 'w') as f:
        f.write(response.text)

# Exemple d'utilisation
if __name__ == "__main__":
    # Encoder
    job_id = encode_file("document.txt", algorithm="fountain", redundancy=1.5)
    print(f"Job créé: {job_id}")

    # Attendre complétion
    result = poll_job(job_id)
    print(f"Stats: {result['stats']}")

    # Télécharger
    download_fasta(job_id, "output.fasta")
    print("Téléchargement terminé!")
```

---

## Webhooks (Futur)

Support de webhooks pour notifications de complétion de jobs (planifié).

```http
POST {webhook_url} HTTP/1.1
Content-Type: application/json

{
  "event": "job.complete",
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "complete",
  "result": {
    "download_url": "/download/fasta/550e8400-e29b-41d4-a716-446655440000"
  }
}
```

---

## Support et Bugs

Pour signaler un bug ou demander une fonctionnalité:
- Issues GitHub: https://github.com/duan78/DNAproof/issues
- Documentation complète: voir `/docs`
