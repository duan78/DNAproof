//! Routes de l'API web

use actix_multipart::Multipart;
use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use chrono::Utc;
use futures::StreamExt;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::models::{
    AppState, DecodeRequest, DecodeResponse, EncodeRequest, EncodeResponse, ErrorResponse,
    JobStatus, JOB_RETENTION,
};

pub mod streaming_routes;
pub use streaming_routes::api_encode_stream;

/// Dossier de stockage des résultats (FASTA encodés, fichiers décodés)
const UPLOAD_DIR: &str = "uploads";

/// Valide qu'un job_id est un UUID bien formé.
///
/// Les job_ids sont insérés dans des chemins de fichiers et des en-têtes
/// HTTP : sans cette validation, un job_id comme `..%2F..%2Fx` permettait
/// une traversée de chemin (lecture de fichiers arbitraires) ou une
/// injection d'en-tête (panic du handler via insert_header).
fn is_valid_job_id(job_id: &str) -> bool {
    Uuid::parse_str(job_id).is_ok()
}

/// Route pour la page d'accueil
#[get("/")]
pub async fn index(data: web::Data<AppState>) -> impl Responder {
    let mut ctx = tera::Context::new();
    ctx.insert("title", "ADN Data Storage");
    ctx.insert("version", env!("CARGO_PKG_VERSION"));

    match data.tera.render("index.html", &ctx) {
        Ok(rendered) => HttpResponse::Ok().content_type("text/html").body(rendered),
        Err(e) => {
            error!("Erreur de rendu du template: {}", e);
            HttpResponse::InternalServerError()
                .json(ErrorResponse::new("Erreur de rendu".to_string(), 500))
        }
    }
}

/// Route pour la page d'encodage
#[get("/encode")]
pub async fn encode_page(data: web::Data<AppState>) -> impl Responder {
    let mut ctx = tera::Context::new();
    ctx.insert("title", "Encoder en ADN");

    match data.tera.render("encode.html", &ctx) {
        Ok(rendered) => HttpResponse::Ok().content_type("text/html").body(rendered),
        Err(e) => {
            error!("Erreur de rendu du template encode: {}", e);
            HttpResponse::InternalServerError()
                .json(ErrorResponse::new("Erreur de rendu".to_string(), 500))
        }
    }
}

/// Route pour la page de décodage
#[get("/decode")]
pub async fn decode_page(data: web::Data<AppState>) -> impl Responder {
    let mut ctx = tera::Context::new();
    ctx.insert("title", "Décoder depuis ADN");

    match data.tera.render("decode.html", &ctx) {
        Ok(rendered) => HttpResponse::Ok().content_type("text/html").body(rendered),
        Err(e) => {
            error!("Erreur de rendu du template decode: {}", e);
            HttpResponse::InternalServerError()
                .json(ErrorResponse::new("Erreur de rendu".to_string(), 500))
        }
    }
}

/// Route pour l'API d'encodage
#[post("/api/encode")]
pub async fn api_encode(
    data: web::Data<AppState>,
    mut payload: Multipart,
    _req: HttpRequest,
) -> impl Responder {
    info!("Nouvelle requête d'encodage");

    let job_id = Uuid::new_v4().to_string();

    // Créer un nouveau job (et évacuer les anciens jobs terminés)
    {
        let mut jobs = data.jobs.write().await;
        evict_expired_jobs_with_lock(&mut jobs).await;
        jobs.insert(job_id.clone(), crate::models::JobState::new(job_id.clone()));

        if let Some(job) = jobs.get_mut(&job_id) {
            job.status = JobStatus::Processing;
            job.updated_at = Utc::now();
        }
    }

    // Traiter le fichier uploadé AVANT de spawner (Multipart n'est pas Send).
    // Les champs texte du formulaire (algorithm, redundancy, compression,
    // chunk_size) alimentent la configuration de l'encodeur.
    let upload_limit = data.config.server.upload_limit;
    let mut request = EncodeRequest::default();
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(item) = payload.next().await {
        let mut field = match item {
            Ok(f) => f,
            Err(e) => {
                error!("Erreur de champ: {}", e);
                return HttpResponse::BadRequest()
                    .json(ErrorResponse::new(format!("Erreur de champ: {}", e), 400));
            }
        };

        let Some(content_disposition) = field.content_disposition() else {
            continue;
        };

        // Extraire les infos de disposition en owned pour libérer l'emprunt
        // de `field` avant d'itérer son contenu.
        let (filename, field_name) = (
            content_disposition.get_filename().map(str::to_string),
            content_disposition.get_name().unwrap_or("").to_string(),
        );

        if let Some(filename) = filename {
            let mut buf: Vec<u8> = Vec::new();
            while let Some(chunk_result) = field.next().await {
                let chunk = match chunk_result {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Erreur de chunk: {}", e);
                        return HttpResponse::BadRequest()
                            .json(ErrorResponse::new(format!("Erreur de chunk: {}", e), 400));
                    }
                };
                buf.extend_from_slice(&chunk);
                if buf.len() > upload_limit {
                    return HttpResponse::PayloadTooLarge().json(ErrorResponse::new(
                        format!("Fichier trop volumineux (limite: {} octets)", upload_limit),
                        413,
                    ));
                }
            }
            if !filename.is_empty() {
                file_data = Some(buf);
            }
        } else {
            // Champ texte du formulaire
            let mut text = String::new();
            while let Some(chunk_result) = field.next().await {
                let chunk = match chunk_result {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Erreur de chunk: {}", e);
                        return HttpResponse::BadRequest()
                            .json(ErrorResponse::new(format!("Erreur de chunk: {}", e), 400));
                    }
                };
                text.push_str(&String::from_utf8_lossy(&chunk));
            }
            match field_name.as_str() {
                "algorithm" => request.algorithm = Some(text),
                "redundancy" => request.redundancy = text.trim().parse().ok(),
                "compression" => {
                    let enabled = text.trim() == "true" || text.trim() == "on";
                    request.compression = Some(enabled);
                }
                "chunk_size" => request.chunk_size = text.trim().parse().ok(),
                _ => {}
            }
        }
    }

    let Some(file_data) = file_data.filter(|d| !d.is_empty()) else {
        return HttpResponse::BadRequest()
            .json(ErrorResponse::new("Aucun fichier fourni".to_string(), 400));
    };

    // Traiter l'encodage en arrière-plan
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();

    tokio::spawn(async move {
        let result = process_encode_data_with_progress(
            &file_data,
            &data_clone,
            job_id_clone.clone(),
            adn_core::EncoderConfig::from(request),
        )
        .await;

        // Mettre à jour le job avec le résultat
        let mut jobs = data_clone.jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id_clone) {
            match result {
                Ok(stats) => {
                    job.status = JobStatus::Complete;
                    job.progress = Some(1.0); // 100% complete
                    job.result = Some(crate::models::JobResult {
                        download_url: Some(format!("/download/fasta/{}", job_id_clone)),
                        stats: Some(stats),
                        sequences: None,
                    });
                }
                Err(e) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("Erreur d'encodage: {}", e));
                }
            }
            job.updated_at = Utc::now();
        }
    });

    HttpResponse::Accepted().json(EncodeResponse {
        job_id,
        status: JobStatus::Processing,
        message: "Encodage en cours".to_string(),
    })
}

/// Traite les données d'encodage avec mises à jour de progression
async fn process_encode_data_with_progress(
    file_data: &[u8],
    data: &web::Data<AppState>,
    job_id: String,
    encoder_config: adn_core::EncoderConfig,
) -> Result<crate::models::EncodingStats, String> {
    // Envoyer la progression initiale
    if let Some(ref tx) = data.progress_tx {
        let _ = tx.send(crate::models::ProgressMessage {
            job_id: job_id.clone(),
            progress: 0.0,
        });
    }

    let encoder = adn_core::Encoder::new(encoder_config)
        .map_err(|e| format!("Erreur d'initialisation de l'encodeur: {}", e))?;

    // L'encodage est CPU-bound : l'exécuter sur le pool de threads de blocage
    // pour ne pas geler les workers async du serveur.
    let encode_start = std::time::Instant::now();

    let sequences = web::block({
        let file_data = file_data.to_vec();
        move || encoder.encode(&file_data)
    })
    .await
    .map_err(|e| format!("Erreur d'encodage (blocking): {}", e))?
    .map_err(|e| format!("Erreur d'encodage: {}", e))?;

    // Envoyer la progression à 90% avant de sauvegarder
    if let Some(ref tx) = data.progress_tx {
        let _ = tx.send(crate::models::ProgressMessage {
            job_id: job_id.clone(),
            progress: 0.9,
        });
    }

    let encoding_time = encode_start.elapsed().as_millis() as u64;

    // Calculer les statistiques
    let total_length: usize = sequences.iter().map(|s| s.bases.len()).sum();
    let avg_length = total_length as f64 / sequences.len() as f64;

    let gc_count: usize = sequences
        .iter()
        .flat_map(|s| s.bases.iter())
        .filter(|b| b.is_gc())
        .count();

    let gc_ratio = gc_count as f64 / total_length as f64;
    let bits_per_base = (file_data.len() * 8) as f64 / total_length as f64;
    let compression_ratio = file_data.len() as f64 / total_length as f64;

    // Sauvegarder dans la base de données si activée
    save_sequences_to_db(data, &sequences).await;

    // Sauvegarder le fichier FASTA
    save_fasta_file(&sequences, &job_id)
        .await
        .map_err(|e| format!("Erreur de sauvegarde FASTA: {}", e))?;

    Ok(crate::models::EncodingStats {
        sequence_count: sequences.len(),
        avg_length,
        gc_ratio,
        bits_per_base,
        file_size: file_data.len(),
        encoded_size: total_length,
        compression_ratio,
        encoding_time_ms: encoding_time,
    })
}

/// Sauvegarde les séquences en base si la base est configurée et disponible
async fn save_sequences_to_db(data: &web::Data<AppState>, sequences: &[adn_core::DnaSequence]) {
    if let Some(db) = &data.database {
        if let Ok(pool) = db.pool() {
            let repo = adn_storage::SequenceRepository::new(std::sync::Arc::new(pool.clone()));

            for seq in sequences {
                if let Err(e) = repo.save_sequence(seq).await {
                    error!("Erreur de sauvegarde dans la base de données: {}", e);
                }
            }
        } else {
            error!("Base de données activée mais le pool n'est pas initialisé");
        }
    }
}

/// Éviction inline avec verrou déjà acquis (évite un deadlock de RwLock)
async fn evict_expired_jobs_with_lock(
    jobs: &mut std::collections::HashMap<String, crate::models::JobState>,
) {
    let now = Utc::now();
    let expired: Vec<String> = jobs
        .iter()
        .filter(|(_, job)| {
            let finished = matches!(job.status, JobStatus::Complete | JobStatus::Failed);
            finished && now.signed_duration_since(job.updated_at) > JOB_RETENTION
        })
        .map(|(id, _)| id.clone())
        .collect();

    if expired.is_empty() {
        return;
    }

    for id in &expired {
        jobs.remove(id);
    }

    // Best-effort : supprimer les fichiers associés
    for id in &expired {
        for ext in ["fasta", "decoded"] {
            let path = std::path::Path::new(UPLOAD_DIR).join(format!("{}.{}", id, ext));
            if let Err(e) = tokio::fs::remove_file(&path).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!("Impossible de supprimer {}: {}", path.display(), e);
                }
            }
        }
    }
}

/// Route pour l'API de décodage
#[post("/api/decode")]
pub async fn api_decode(data: web::Data<AppState>, mut payload: Multipart) -> impl Responder {
    info!("Nouvelle requête de décodage");

    let job_id = Uuid::new_v4().to_string();

    // Créer un nouveau job (et évacuer les anciens jobs terminés)
    {
        let mut jobs = data.jobs.write().await;
        evict_expired_jobs_with_lock(&mut jobs).await;
        jobs.insert(job_id.clone(), crate::models::JobState::new(job_id.clone()));

        if let Some(job) = jobs.get_mut(&job_id) {
            job.status = JobStatus::Processing;
            job.updated_at = Utc::now();
        }
    }

    // Traiter le fichier uploadé AVANT de spawner (Multipart n'est pas Send)
    let upload_limit = data.config.server.upload_limit;
    let mut request = DecodeRequest::default();
    let mut fasta_data: Option<Vec<u8>> = None;

    while let Some(item) = payload.next().await {
        let mut field = match item {
            Ok(f) => f,
            Err(e) => {
                error!("Erreur de champ: {}", e);
                return HttpResponse::BadRequest()
                    .json(ErrorResponse::new(format!("Erreur de champ: {}", e), 400));
            }
        };

        let Some(content_disposition) = field.content_disposition() else {
            continue;
        };

        // Extraire les infos de disposition en owned pour libérer l'emprunt
        // de `field` avant d'itérer son contenu.
        let (filename, field_name) = (
            content_disposition.get_filename().map(str::to_string),
            content_disposition.get_name().unwrap_or("").to_string(),
        );

        if let Some(filename) = filename {
            let mut buf: Vec<u8> = Vec::new();
            while let Some(chunk_result) = field.next().await {
                let chunk = match chunk_result {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Erreur de chunk: {}", e);
                        return HttpResponse::BadRequest()
                            .json(ErrorResponse::new(format!("Erreur de chunk: {}", e), 400));
                    }
                };
                buf.extend_from_slice(&chunk);
                if buf.len() > upload_limit {
                    return HttpResponse::PayloadTooLarge().json(ErrorResponse::new(
                        format!("Fichier trop volumineux (limite: {} octets)", upload_limit),
                        413,
                    ));
                }
            }
            if !filename.is_empty() {
                fasta_data = Some(buf);
            }
        } else {
            let mut text = String::new();
            while let Some(chunk_result) = field.next().await {
                let chunk = match chunk_result {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Erreur de chunk: {}", e);
                        return HttpResponse::BadRequest()
                            .json(ErrorResponse::new(format!("Erreur de chunk: {}", e), 400));
                    }
                };
                text.push_str(&String::from_utf8_lossy(&chunk));
            }
            if field_name == "auto_decompress" {
                request.auto_decompress = Some(text.trim() == "true" || text.trim() == "on");
            }
        }
    }

    let Some(fasta_data) = fasta_data.filter(|d| !d.is_empty()) else {
        return HttpResponse::BadRequest()
            .json(ErrorResponse::new("Aucun fichier fourni".to_string(), 400));
    };

    // Traiter le décodage en arrière-plan
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();

    tokio::spawn(async move {
        let result = process_decode_data(
            &fasta_data,
            &data_clone,
            job_id_clone.clone(),
            adn_core::DecoderConfig::from(request),
        )
        .await;

        // Mettre à jour le job avec le résultat
        let mut jobs = data_clone.jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id_clone) {
            match result {
                Ok(_) => {
                    job.status = JobStatus::Complete;
                    job.result = Some(crate::models::JobResult {
                        download_url: Some(format!("/download/{}", job_id_clone)),
                        stats: None,
                        sequences: None,
                    });
                }
                Err(e) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("Erreur de décodage: {}", e));
                }
            }
            job.updated_at = Utc::now();
        }
    });

    HttpResponse::Accepted().json(DecodeResponse {
        job_id,
        status: JobStatus::Processing,
        message: "Décodage en cours".to_string(),
    })
}

/// Traite les données de décodage
async fn process_decode_data(
    fasta_data: &[u8],
    data: &web::Data<AppState>,
    job_id: String,
    decoder_config: adn_core::DecoderConfig,
) -> Result<(), String> {
    // Parser le fichier FASTA
    let sequences =
        parse_fasta(fasta_data).map_err(|e| format!("Erreur de parsing FASTA: {}", e))?;

    if sequences.is_empty() {
        return Err("Aucune séquence ADN fournie".to_string());
    }

    // Décoder les séquences (CPU-bound → thread de blocage)
    let decoder = adn_core::Decoder::new(decoder_config);
    let decoded_data = web::block({
        let sequences = sequences.clone();
        move || decoder.decode(&sequences)
    })
    .await
    .map_err(|e| format!("Erreur de décodage (blocking): {}", e))?
    .map_err(|e| format!("Erreur de décodage: {}", e))?;

    // Sauvegarder le résultat pour téléchargement
    save_decoded_result(data, &job_id, &decoded_data)
        .await
        .map_err(|e| format!("Erreur de sauvegarde du résultat: {}", e))?;

    Ok(())
}

/// Parse un fichier FASTA
fn parse_fasta(data: &[u8]) -> Result<Vec<adn_core::DnaSequence>, String> {
    let content = String::from_utf8_lossy(data);

    // Parser chaque enregistrement FASTA via DnaSequence::from_fasta, qui
    // conserve les métadonnées de l'en-tête (scheme:, seed:, chunk:) —
    // indispensables au décodeur pour router et reconstruire les droplets.
    // L'ancien parsing les ignorait (seed=0, scheme inconnu) : le décodage
    // web échouait systématiquement.
    content
        .split('>')
        .filter(|bloc| !bloc.trim().is_empty())
        .map(|bloc| {
            let record = format!(">{}", bloc);
            adn_core::DnaSequence::from_fasta(&record)
                .map_err(|e| format!("Enregistrement FASTA invalide: {}", e))
        })
        .collect()
}

/// Sauvegarde les séquences au format FASTA
pub(crate) async fn save_fasta_file(
    sequences: &[adn_core::DnaSequence],
    job_id: &str,
) -> Result<(), String> {
    let upload_dir = std::path::Path::new(UPLOAD_DIR);

    if !upload_dir.exists() {
        std::fs::create_dir_all(upload_dir)
            .map_err(|e| format!("Erreur de création du dossier: {}", e))?;
    }

    let file_path = upload_dir.join(format!("{}.fasta", job_id));

    // Générer le contenu FASTA
    let fasta_content: String = sequences.iter().map(|seq| seq.to_fasta()).collect();

    tokio::fs::write(&file_path, fasta_content)
        .await
        .map_err(|e| format!("Erreur d'écriture du fichier FASTA: {}", e))?;

    Ok(())
}

/// Sauvegarde le résultat décodé
async fn save_decoded_result(
    _data: &web::Data<AppState>,
    job_id: &str,
    decoded_data: &[u8],
) -> Result<(), String> {
    let upload_dir = std::path::Path::new(UPLOAD_DIR);

    if !upload_dir.exists() {
        std::fs::create_dir_all(upload_dir)
            .map_err(|e| format!("Erreur de création du dossier: {}", e))?;
    }

    let file_path = upload_dir.join(format!("{}.decoded", job_id));

    tokio::fs::write(&file_path, decoded_data)
        .await
        .map_err(|e| format!("Erreur d'écriture du fichier: {}", e))?;

    Ok(())
}

/// Route pour vérifier l'état d'un job
#[get("/api/jobs/{job_id}")]
pub async fn job_status(data: web::Data<AppState>, job_id: web::Path<String>) -> impl Responder {
    if !is_valid_job_id(&job_id) {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "Identifiant de job invalide".to_string(),
            400,
        ));
    }

    let jobs = data.jobs.read().await;

    match jobs.get(job_id.as_ref()) {
        Some(job) => HttpResponse::Ok().json(job),
        None => {
            HttpResponse::NotFound().json(ErrorResponse::new("Job non trouvé".to_string(), 404))
        }
    }
}

/// Route pour télécharger un résultat
#[get("/download/{job_id}")]
pub async fn download_result(
    _data: web::Data<AppState>,
    job_id: web::Path<String>,
) -> impl Responder {
    // Validation stricte : le job_id est utilisé pour construire un chemin de
    // fichier (bloque la traversée de répertoire) et injecté dans un en-tête.
    if !is_valid_job_id(&job_id) {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "Identifiant de job invalide".to_string(),
            400,
        ));
    }

    let file_path = std::path::Path::new(UPLOAD_DIR).join(format!("{}.decoded", job_id.as_ref()));

    match tokio::fs::read(&file_path).await {
        Ok(data) => HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(data),
        Err(_) => {
            HttpResponse::NotFound().json(ErrorResponse::new("Fichier non trouvé".to_string(), 404))
        }
    }
}

/// Route pour télécharger un fichier FASTA
#[get("/download/fasta/{job_id}")]
pub async fn download_fasta(job_id: web::Path<String>) -> impl Responder {
    if !is_valid_job_id(&job_id) {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "Identifiant de job invalide".to_string(),
            400,
        ));
    }

    // job_id validé comme UUID : safe pour un en-tête Content-Disposition
    let file_path = std::path::Path::new(UPLOAD_DIR).join(format!("{}.fasta", job_id.as_ref()));

    match tokio::fs::read(&file_path).await {
        Ok(data) => HttpResponse::Ok()
            .content_type("text/x-fasta")
            .insert_header((
                "Content-Disposition",
                format!("attachment; filename=\"{}.fasta\"", job_id.as_ref()),
            ))
            .body(data),
        Err(_) => HttpResponse::NotFound().json(ErrorResponse::new(
            "Fichier FASTA non trouvé".to_string(),
            404,
        )),
    }
}

/// Route pour la santé de l'API
#[get("/health")]
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now(),
        "version": env!("CARGO_PKG_VERSION")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_id_validation() {
        assert!(is_valid_job_id("0a1b2c3d-1111-4222-8333-444455556666"));
        assert!(!is_valid_job_id("../../etc/passwd"));
        assert!(!is_valid_job_id("..%2F..%2Fsecret"));
        assert!(!is_valid_job_id("aa\r\nbb"));
        assert!(!is_valid_job_id(""));
        assert!(!is_valid_job_id("not-a-uuid"));
    }

    #[test]
    fn test_parse_fasta_roundtrip() {
        let fasta = b">seq1|scheme:test\nACGTACGT\n>seq2\nTTTTGGGG\n";
        let seqs = parse_fasta(fasta).unwrap();
        assert_eq!(seqs.len(), 2);
        assert_eq!(seqs[0].bases.len(), 8);
        assert_eq!(seqs[1].bases.len(), 8);
    }
}
