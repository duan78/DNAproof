//! Routes de l'API web

use actix_web::{web, HttpResponse, Responder, HttpRequest, get, post};
use actix_multipart::Multipart;
use futures::StreamExt;
use std::path::PathBuf;
use tracing::{info, error, instrument};
use uuid::Uuid;
use chrono::Utc;

use crate::models::{AppState, EncodeRequest, EncodeResponse, DecodeRequest, DecodeResponse, JobStatus, ErrorResponse};

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
            HttpResponse::InternalServerError().json(ErrorResponse::new(
                "Erreur de rendu".to_string(),
                500
            ))
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
            HttpResponse::InternalServerError().json(ErrorResponse::new(
                "Erreur de rendu".to_string(),
                500
            ))
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
            HttpResponse::InternalServerError().json(ErrorResponse::new(
                "Erreur de rendu".to_string(),
                500
            ))
        }
    }
}

/// Route pour l'API d'encodage
#[post("/api/encode")]
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
        job.updated_at = Utc::now();
    }

    drop(jobs); // Libérer le verrou

    // Traiter le fichier uploadé AVANT de spawner (Multipart n'est pas Send)
    let mut file_data = Vec::new();
    let mut file_name = None;

    while let Some(item) = payload.next().await {
        let field = match item {
            Ok(f) => f,
            Err(e) => {
                error!("Erreur de champ: {}", e);
                return HttpResponse::BadRequest().json(ErrorResponse::new(
                    format!("Erreur de champ: {}", e),
                    400
                ));
            }
        };

        if let Some(content_disposition) = field.content_disposition() {
            if let Some(name) = content_disposition.get_filename() {
                file_name = Some(name.to_string());

                let mut field = field;
                while let Some(chunk_result) = field.next().await {
                    let data = match chunk_result {
                        Ok(d) => d,
                        Err(e) => {
                            error!("Erreur de chunk: {}", e);
                            return HttpResponse::BadRequest().json(ErrorResponse::new(
                                format!("Erreur de chunk: {}", e),
                                400
                            ));
                        }
                    };
                    file_data.extend_from_slice(&data);
                }
            }
        }
    }

    if file_data.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "Aucun fichier fourni".to_string(),
            400
        ));
    }

    // Traiter l'encodage en arrière-plan
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();
    let file_size = file_data.len();

    tokio::spawn(async move {
        let result = process_encode_data_with_progress(
            &file_data,
            &data_clone,
            job_id_clone.clone(),
            file_size,
        ).await;

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
    file_size: usize,
) -> Result<crate::models::EncodingStats, String> {
    // Envoyer la progression initiale
    if let Some(ref tx) = data.progress_tx {
        let _ = tx.send(crate::models::ProgressMessage {
            job_id: job_id.clone(),
            progress: 0.0,
        });
    }

    // Encoder les données
    let start_time = std::time::Instant::now();
    let encoder = adn_core::Encoder::new(adn_core::EncoderConfig::default())
        .map_err(|e| format!("Erreur d'initialisation de l'encodeur: {}", e))?;

    // Pour les fichiers volumineux, simuler une progression
    // (l'encodeur actuel est synchrone et ne fournit pas de callbacks)
    if file_size > 100_000 { // > 100KB
        // Envoyer une progression de 50% au milieu
        if let Some(ref tx) = data.progress_tx {
            let _ = tx.send(crate::models::ProgressMessage {
                job_id: job_id.clone(),
                progress: 0.5,
            });
        }
    }

    let sequences = encoder.encode(file_data)
        .map_err(|e| format!("Erreur d'encodage: {}", e))?;

    // Envoyer la progression à 90% avant de sauvegarder
    if let Some(ref tx) = data.progress_tx {
        let _ = tx.send(crate::models::ProgressMessage {
            job_id: job_id.clone(),
            progress: 0.9,
        });
    }

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
        let pool = db.pool().unwrap();
        let mut repo = adn_storage::SequenceRepository::new(std::sync::Arc::new(pool.clone()));

        for seq in &sequences {
            if let Err(e) = repo.save_sequence(seq).await {
                error!("Erreur de sauvegarde dans la base de données: {}", e);
            }
        }
    }

    // Sauvegarder le fichier FASTA
    save_fasta_file(&sequences, &job_id).await
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

/// Route pour l'API de décodage
#[post("/api/decode")]
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
        job.updated_at = Utc::now();
    }

    drop(jobs); // Libérer le verrou

    // Traiter le fichier uploadé AVANT de spawner (Multipart n'est pas Send)
    let mut fasta_data = Vec::new();

    while let Some(item) = payload.next().await {
        let field = match item {
            Ok(f) => f,
            Err(e) => {
                error!("Erreur de champ: {}", e);
                return HttpResponse::BadRequest().json(ErrorResponse::new(
                    format!("Erreur de champ: {}", e),
                    400
                ));
            }
        };

        if let Some(content_disposition) = field.content_disposition() {
            if let Some(_name) = content_disposition.get_filename() {
                let mut field = field;
                while let Some(chunk_result) = field.next().await {
                    let data = match chunk_result {
                        Ok(d) => d,
                        Err(e) => {
                            error!("Erreur de chunk: {}", e);
                            return HttpResponse::BadRequest().json(ErrorResponse::new(
                                format!("Erreur de chunk: {}", e),
                                400
                            ));
                        }
                    };
                    fasta_data.extend_from_slice(&data);
                }
            }
        }
    }

    if fasta_data.is_empty() {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "Aucun fichier fourni".to_string(),
            400
        ));
    }

    // Traiter le décodage en arrière-plan
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();

    tokio::spawn(async move {
        let result = process_decode_data(&fasta_data, &data_clone, job_id_clone.clone()).await;

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
) -> Result<(), String> {
    // Parser le fichier FASTA
    let sequences = parse_fasta(fasta_data)
        .map_err(|e| format!("Erreur de parsing FASTA: {}", e))?;

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
    let mut current_id: Option<String> = None;

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

/// Sauvegarde les séquences au format FASTA
async fn save_fasta_file(
    sequences: &[adn_core::DnaSequence],
    job_id: &str,
) -> Result<(), String> {
    let upload_dir = std::path::Path::new("uploads");

    if !upload_dir.exists() {
        std::fs::create_dir_all(upload_dir)
            .map_err(|e| format!("Erreur de création du dossier: {}", e))?;
    }

    let file_path = upload_dir.join(format!("{}.fasta", job_id));

    // Générer le contenu FASTA
    let fasta_content: String = sequences.iter()
        .map(|seq| seq.to_fasta())
        .collect();

    tokio::fs::write(&file_path, fasta_content)
        .await
        .map_err(|e| format!("Erreur d'écriture du fichier FASTA: {}", e))?;

    Ok(())
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
#[get("/api/jobs/{job_id}")]
pub async fn job_status(
    data: web::Data<AppState>,
    job_id: web::Path<String>,
) -> impl Responder {
    let jobs = data.jobs.read().await;

    match jobs.get(job_id.as_ref()) {
        Some(job) => HttpResponse::Ok().json(job),
        None => HttpResponse::NotFound().json(ErrorResponse::new(
            "Job non trouvé".to_string(),
            404
        )),
    }
}

/// Route pour télécharger un résultat
#[get("/download/{job_id}")]
pub async fn download_result(
    data: web::Data<AppState>,
    job_id: web::Path<String>,
) -> impl Responder {
    let file_path = std::path::Path::new("uploads").join(format!("{}.decoded", job_id.as_ref()));

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

/// Route pour télécharger un fichier FASTA
#[get("/download/fasta/{job_id}")]
pub async fn download_fasta(
    job_id: web::Path<String>,
) -> impl Responder {
    let file_path = std::path::Path::new("uploads").join(format!("{}.fasta", job_id.as_ref()));

    match tokio::fs::read(&file_path).await {
        Ok(data) => HttpResponse::Ok()
            .content_type("text/x-fasta")
            .insert_header(("Content-Disposition", format!("attachment; filename=\"{}.fasta\"", job_id.as_ref())))
            .body(data),
        Err(_) => HttpResponse::NotFound().json(ErrorResponse::new(
            "Fichier FASTA non trouvé".to_string(),
            404
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