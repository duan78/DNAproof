//! Routes de l'API web

use actix_web::{web, HttpResponse, Responder, HttpRequest};
use actix_multipart::Multipart;
use futures::{StreamExt, TryStreamExt};
use std::path::PathBuf;
use tracing::{info, error, instrument};
use uuid::Uuid;

use crate::models::{AppState, EncodeRequest, EncodeResponse, DecodeRequest, DecodeResponse, JobStatus, ErrorResponse};

/// Route pour la page d'accueil
#[instrument]
pub async fn index(data: web::Data<AppState>) -> impl Responder {
    let mut ctx = tera::Context::new();
    ctx.insert("title", "ADN Data Storage");
    ctx.insert("version", env!("CARGO_PKG_VERSION"));

    match data.tera.render("index.html", &ctx) {
        Ok(rendered) => HttpResponse::Ok().content_type("text/html").body(rendered),
        Err(e) => {
            error!("Erreur de rendu du template: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse::new(
                "Erreur de rendu".to_string(),
                500
            ))
        }
    }
}

/// Route pour la page d'encodage
#[instrument]
pub async fn encode_page(data: web::Data<AppState>) -> impl Responder {
    let mut ctx = tera::Context::new();
    ctx.insert("title", "Encoder en ADN");

    match data.tera.render("encode.html", &ctx) {
        Ok(rendered) => HttpResponse::Ok().content_type("text/html").body(rendered),
        Err(e) => {
            error!("Erreur de rendu du template encode: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse::new(
                "Erreur de rendu".to_string(),
                500
            ))
        }
    }
}

/// Route pour la page de décodage
#[instrument]
pub async fn decode_page(data: web::Data<AppState>) -> impl Responder {
    let mut ctx = tera::Context::new();
    ctx.insert("title", "Décoder depuis ADN");

    match data.tera.render("decode.html", &ctx) {
        Ok(rendered) => HttpResponse::Ok().content_type("text/html").body(rendered),
        Err(e) => {
            error!("Erreur de rendu du template decode: {}", e);
            HttpResponse::InternalServerError().json(ErrorResponse::new(
                "Erreur de rendu".to_string(),
                500
            ))
        }
    }
}

/// Route pour l'API d'encodage
#[instrument]
pub async fn api_encode(
    data: web::Data<AppState>,
    mut payload: Multipart,
    req: HttpRequest,
) -> impl Responder {
    info!("Nouvelle requête d'encodage");

    let job_id = Uuid::new_v4().to_string();
    
    // Créer un nouveau job
    let mut jobs = data.jobs.write().await;
    jobs.insert(job_id.clone(), crate::models::JobState::new(job_id.clone()));
    
    // Mettre à jour le statut
    if let Some(job) = jobs.get_mut(&job_id) {
        job.status = JobStatus::Processing;
        job.updated_at = chrono::Utc::now();
    }
    
    drop(jobs); // Libérer le verrou

    // Traiter le fichier en arrière-plan
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();

    tokio::spawn(async move {
        let result = process_encode_upload(&mut payload, &data_clone, job_id_clone.clone()).await;
        
        // Mettre à jour le job avec le résultat
        let mut jobs = data_clone.jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id_clone) {
            match result {
                Ok(stats) => {
                    job.status = JobStatus::Complete;
                    job.result = Some(crate::models::JobResult {
                        download_url: Some(format!("/download/{}", job_id_clone)),
                        stats: Some(stats),
                        sequences: None,
                    });
                }
                Err(e) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("Erreur d'encodage: {}", e));
                }
            }
            job.updated_at = chrono::Utc::now();
        }
    });

    HttpResponse::Accepted().json(EncodeResponse {
        job_id,
        status: JobStatus::Processing,
        message: "Encodage en cours".to_string(),
    })
}

/// Traite l'upload et l'encodage
async fn process_encode_upload(
    payload: &mut Multipart,
    data: &web::Data<AppState>,
    job_id: String,
) -> Result<crate::models::EncodingStats, String> {
    let mut file_data = Vec::new();
    let mut file_name = None;

    while let Some(item) = payload.next().await {
        let mut field = item.map_err(|e| format!("Erreur de champ: {}", e))?;
        
        if let Some(content_disposition) = field.content_disposition() {
            if let Some(name) = content_disposition.get_filename() {
                file_name = Some(name.to_string());
                
                while let Some(chunk) = field.next().await {
                    let data = chunk.map_err(|e| format!("Erreur de chunk: {}", e))?;
                    file_data.extend_from_slice(&data);
                }
            }
        }
    }

    if file_data.is_empty() {
        return Err("Aucun fichier fourni".to_string());
    }

    // Encoder les données
    let start_time = std::time::Instant::now();
    let encoder = adn_core::Encoder::new(adn_core::EncoderConfig::default())
        .map_err(|e| format!("Erreur d'initialisation de l'encodeur: {}", e))?;

    let sequences = encoder.encode(&file_data)
        .map_err(|e| format!("Erreur d'encodage: {}", e))?;

    let encoding_time = start_time.elapsed().as_millis() as u64;

    // Calculer les statistiques
    let total_length: usize = sequences.iter().map(|s| s.bases.len()).sum();
    let avg_length = total_length as f64 / sequences.len() as f64;
    
    let gc_count: usize = sequences.iter()
        .flat_map(|s| s.bases.iter())
        .filter(|b| b.is_gc())
        .count();
    
    let gc_ratio = gc_count as f64 / total_length as f64;
    let bits_per_base = (file_data.len() * 8) as f64 / total_length as f64;
    let compression_ratio = file_data.len() as f64 / total_length as f64;

    // Sauvegarder dans la base de données si activé
    if let Some(db) = &data.database {
        let mut repo = adn_storage::SequenceRepository::new(db.pool().unwrap().clone());
        
        for seq in &sequences {
            if let Err(e) = repo.save_sequence(seq).await {
                error!("Erreur de sauvegarde dans la base de données: {}", e);
            }
        }
    }

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

/// Route pour l'API de décodage
#[instrument]
pub async fn api_decode(
    data: web::Data<AppState>,
    mut payload: Multipart,
) -> impl Responder {
    info!("Nouvelle requête de décodage");

    let job_id = Uuid::new_v4().to_string();
    
    // Créer un nouveau job
    let mut jobs = data.jobs.write().await;
    jobs.insert(job_id.clone(), crate::models::JobState::new(job_id.clone()));
    
    // Mettre à jour le statut
    if let Some(job) = jobs.get_mut(&job_id) {
        job.status = JobStatus::Processing;
        job.updated_at = chrono::Utc::now();
    }
    
    drop(jobs); // Libérer le verrou

    // Traiter le fichier en arrière-plan
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();

    tokio::spawn(async move {
        let result = process_decode_upload(&mut payload, &data_clone, job_id_clone.clone()).await;
        
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
            job.updated_at = chrono::Utc::now();
        }
    });

    HttpResponse::Accepted().json(DecodeResponse {
        job_id,
        status: JobStatus::Processing,
        message: "Décodage en cours".to_string(),
    })
}

/// Traite l'upload et le décodage
async fn process_decode_upload(
    payload: &mut Multipart,
    data: &web::Data<AppState>,
    job_id: String,
) -> Result<(), String> {
    let mut sequences = Vec::new();

    while let Some(item) = payload.next().await {
        let mut field = item.map_err(|e| format!("Erreur de champ: {}", e))?;
        
        if let Some(content_disposition) = field.content_disposition() {
            if let Some(name) = content_disposition.get_filename() {
                let mut file_data = Vec::new();
                
                while let Some(chunk) = field.next().await {
                    let data = chunk.map_err(|e| format!("Erreur de chunk: {}", e))?;
                    file_data.extend_from_slice(&data);
                }
                
                // Parser le fichier FASTA
                let seqs = parse_fasta(&file_data)
                    .map_err(|e| format!("Erreur de parsing FASTA: {}", e))?;
                
                sequences.extend(seqs);
            }
        }
    }

    if sequences.is_empty() {
        return Err("Aucune séquence ADN fournie".to_string());
    }

    // Décoder les séquences
    let decoder = adn_core::Decoder::new(adn_core::DecoderConfig::default());
    let decoded_data = decoder.decode(&sequences)
        .map_err(|e| format!("Erreur de décodage: {}", e))?;

    // Sauvegarder le résultat pour téléchargement
    save_decoded_result(&data, &job_id, &decoded_data).await
        .map_err(|e| format!("Erreur de sauvegarde du résultat: {}", e))?;

    Ok(())
}

/// Parse un fichier FASTA
fn parse_fasta(data: &[u8]) -> Result<Vec<adn_core::DnaSequence>, String> {
    let content = String::from_utf8_lossy(data);
    let mut sequences = Vec::new();
    
    let mut current_seq = String::new();
    let mut current_id = None;
    
    for line in content.lines() {
        if line.starts_with('>') {
            // Sauvegarder la séquence précédente
            if !current_seq.is_empty() && current_id.is_some() {
                sequences.push(parse_sequence(&current_id.unwrap(), &current_seq)?);
                current_seq.clear();
            }
            
            // Nouvelle séquence
            current_id = Some(line[1..].trim().to_string());
        } else {
            current_seq.push_str(line);
        }
    }
    
    // Sauvegarder la dernière séquence
    if !current_seq.is_empty() && current_id.is_some() {
        sequences.push(parse_sequence(&current_id.unwrap(), &current_seq)?);
    }
    
    Ok(sequences)
}

/// Parse une séquence individuelle
fn parse_sequence(id: &str, seq: &str) -> Result<adn_core::DnaSequence, String> {
    let bases = seq.chars()
        .filter_map(|c| match c {
            'A' | 'a' => Some(adn_core::IupacBase::A),
            'C' | 'c' => Some(adn_core::IupacBase::C),
            'G' | 'g' => Some(adn_core::IupacBase::G),
            'T' | 't' => Some(adn_core::IupacBase::T),
            _ => None,
        })
        .collect();
    
    Ok(adn_core::DnaSequence::new(
        bases,
        id.to_string(),
        0,
        seq.len(),
        0,
    ))
}

/// Sauvegarde le résultat décodé
async fn save_decoded_result(
    data: &web::Data<AppState>,
    job_id: &str,
    decoded_data: &[u8],
) -> Result<(), String> {
    let upload_dir = std::path::Path::new("uploads");
    
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
#[instrument]
pub async fn job_status(
    data: web::Data<AppState>,
    job_id: web::Path<String>,
) -> impl Responder {
    let jobs = data.jobs.read().await;
    
    match jobs.get(&job_id) {
        Some(job) => HttpResponse::Ok().json(job),
        None => HttpResponse::NotFound().json(ErrorResponse::new(
            "Job non trouvé".to_string(),
            404
        )),
    }
}

/// Route pour télécharger un résultat
#[instrument]
pub async fn download_result(
    data: web::Data<AppState>,
    job_id: web::Path<String>,
) -> impl Responder {
    let file_path = std::path::Path::new("uploads").join(format!("{}.decoded", job_id));
    
    match tokio::fs::read(&file_path).await {
        Ok(data) => HttpResponse::Ok()
            .content_type("application/octet-stream")
            .body(data),
        Err(_) => HttpResponse::NotFound().json(ErrorResponse::new(
            "Fichier non trouvé".to_string(),
            404
        )),
    }
}

/// Route pour la santé de l'API
#[instrument]
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now(),
        "version": env!("CARGO_PKG_VERSION")
    }))
}