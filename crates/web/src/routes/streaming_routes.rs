//! Routes de streaming pour les gros fichiers

use actix_web::{post, web, HttpResponse, Responder};
use chrono::Utc;
use futures::StreamExt;
use tracing::{error, info};
use uuid::Uuid;

use crate::models::{AppState, EncodeResponse, ErrorResponse, JobStatus};
use adn_core::codec::encoder::CompressionType;
use adn_core::codec::encoder::EncoderType;

/// Route pour l'API d'encodage en streaming (pour les gros fichiers)
#[post("/api/encode/stream")]
pub async fn api_encode_stream(
    data: web::Data<AppState>,
    req: actix_web::HttpRequest,
    payload: web::Payload,
) -> impl Responder {
    info!("Nouvelle requête d'encodage en streaming");

    let job_id = Uuid::new_v4().to_string();

    // Créer un nouveau job
    let mut jobs = data.jobs.write().await;
    jobs.insert(job_id.clone(), crate::models::JobState::new(job_id.clone()));

    // Mettre à jour le statut
    if let Some(job) = jobs.get_mut(&job_id) {
        job.status = JobStatus::Processing;
        job.updated_at = Utc::now();
    }

    drop(jobs); // Libérer le verrou

    // Traiter le streaming directement (sans spawn car Payload n'est pas Send)
    let result = process_streaming_encode(payload, req, &data, job_id.clone()).await;

    // Mettre à jour le job avec le résultat
    let mut jobs = data.jobs.write().await;
    let response = match &result {
        Ok(stats) => {
            if let Some(job) = jobs.get_mut(&job_id) {
                job.status = JobStatus::Complete;
                job.result = Some(crate::models::JobResult {
                    download_url: Some(format!("/download/fasta/{}", job_id)),
                    stats: Some(stats.clone()),
                    sequences: None,
                });
                job.updated_at = Utc::now();
            }

            HttpResponse::Ok().json(EncodeResponse {
                job_id: job_id.clone(),
                status: JobStatus::Complete,
                message: "Encodage en streaming terminé".to_string(),
            })
        }
        Err(err) => {
            if let Some(job) = jobs.get_mut(&job_id) {
                job.status = JobStatus::Failed;
                job.error = Some(format!("Erreur d'encodage en streaming: {}", err));
                job.updated_at = Utc::now();
            }

            HttpResponse::InternalServerError().json(ErrorResponse::new(
                format!("Erreur d'encodage en streaming: {}", err),
                500,
            ))
        }
    };

    response
}

/// Traite les données d'encodage en streaming
async fn process_streaming_encode(
    payload: web::Payload,
    req: actix_web::HttpRequest,
    data: &web::Data<AppState>,
    job_id: String,
) -> Result<crate::models::EncodingStats, String> {
    let start_time = std::time::Instant::now();

    let upload_limit = data.config.server.upload_limit;
    // Taille totale si le client l'a annoncée (Content-Length)
    let total_expected = req
        .headers()
        .get(actix_web::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());

    // Lire le streaming en chunks avec limite de taille
    let mut file_data = Vec::new();
    let mut bytes_received = 0usize;
    let mut next_progress_update = 102_400usize; // progression ~tous les 100KB

    // web::Payload implémente Stream<Item = Result<Bytes, PayloadError>>
    let mut stream = payload;

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(e) => {
                return Err(format!("Erreur de lecture du stream: {}", e));
            }
        };

        file_data.extend_from_slice(&chunk);
        bytes_received += chunk.len();

        if bytes_received > upload_limit {
            return Err(format!(
                "Fichier trop volumineux: {} octets reçus (limite: {} octets)",
                bytes_received, upload_limit
            ));
        }

        // Progression basée sur la taille annoncée par le client si disponible.
        // Sans Content-Length, on ne peut pas calculer un pourcentage honnête :
        // on signale juste des jalons relatifs au volume reçu.
        if bytes_received >= next_progress_update {
            next_progress_update = bytes_received + 102_400;
            if let Some(ref tx) = data.progress_tx {
                let progress = match total_expected {
                    Some(total) if total > 0 => (bytes_received as f64 / total as f64).min(0.9),
                    _ => 0.5,
                };
                let _ = tx.send(crate::models::ProgressMessage {
                    job_id: job_id.clone(),
                    progress,
                });
            }
        }
    }

    if file_data.is_empty() {
        return Err("Aucune donnée reçue".to_string());
    }

    // Traiter l'encodage
    let file_size = file_data.len();

    // Configurer l'encodeur avec des contraintes appropriées pour le streaming
    let config = adn_core::EncoderConfig {
        encoder_type: EncoderType::Fountain,
        chunk_size: 32,
        redundancy: 1.5,
        compression_enabled: true,
        compression_type: CompressionType::Lz4,
        constraints: adn_core::DnaConstraints {
            gc_min: 0.3,
            gc_max: 0.7,
            max_homopolymer: 3,
            max_sequence_length: 150,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
    };

    let encoder = adn_core::Encoder::new(config)
        .map_err(|e| format!("Erreur de création de l'encodeur: {}", e))?;

    // Encodage CPU-bound sur le pool de threads de blocage
    let sequences = web::block(move || encoder.encode(&file_data))
        .await
        .map_err(|e| format!("Erreur d'encodage (blocking): {}", e))?
        .map_err(|e| format!("Erreur d'encodage: {}", e))?;

    let encoding_time = start_time.elapsed().as_millis() as u64;

    // Calculer les statistiques
    let total_length: usize = sequences.iter().map(|s| s.bases.len()).sum();
    let avg_length = total_length as f64 / sequences.len() as f64;

    let gc_count: usize = sequences
        .iter()
        .flat_map(|s| s.bases.iter())
        .filter(|b| b.is_gc())
        .count();

    let gc_ratio = gc_count as f64 / total_length as f64;
    let bits_per_base = (file_size * 8) as f64 / total_length as f64;
    let compression_ratio = file_size as f64 / total_length as f64;

    // Sauvegarder le fichier FASTA
    crate::routes::save_fasta_file(&sequences, &job_id)
        .await
        .map_err(|e| format!("Erreur de sauvegarde FASTA: {}", e))?;

    // Sauvegarder dans la base de données si activée
    if let Some(db) = &data.database {
        if let Ok(pool) = db.pool() {
            let repo = adn_storage::SequenceRepository::new(std::sync::Arc::new(pool.clone()));

            for seq in &sequences {
                if let Err(e) = repo.save_sequence(seq).await {
                    error!("Erreur de sauvegarde dans la base de données: {}", e);
                }
            }
        } else {
            error!("Base de données activée mais le pool n'est pas initialisé");
        }
    }

    Ok(crate::models::EncodingStats {
        sequence_count: sequences.len(),
        avg_length,
        gc_ratio,
        bits_per_base,
        file_size,
        encoded_size: total_length,
        compression_ratio,
        encoding_time_ms: encoding_time,
    })
}
