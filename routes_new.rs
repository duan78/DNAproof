// Route de streaming pour les gros fichiers

/// Route pour l'API d'encodage en streaming (pour les gros fichiers)
#[post("/api/encode/stream")]
pub async fn api_encode_stream(
    data: web::Data<AppState>,
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

    // Traiter le streaming
    let data_clone = data.clone();
    let job_id_clone = job_id.clone();

    tokio::spawn(async move {
        let result = process_streaming_encode(payload, &data_clone, job_id_clone.clone()).await;

        // Mettre à jour le job avec le résultat
        let mut jobs = data_clone.jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id_clone) {
            match result {
                Ok(stats) => {
                    job.status = JobStatus::Complete;
                    job.result = Some(crate::models::JobResult {
                        download_url: Some(format!("/download/fasta/{}", job_id_clone)),
                        stats: Some(stats),
                        sequences: None,
                    });
                }
                Err(e) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("Erreur d'encodage en streaming: {}", e));
                }
            }
            job.updated_at = Utc::now();
        }
    });

    HttpResponse::Accepted().json(EncodeResponse {
        job_id,
        status: JobStatus::Processing,
        message: "Encodage en streaming en cours".to_string(),
    })
}

/// Traite les données d'encodage en streaming
async fn process_streaming_encode(
    payload: web::Payload,
    data: &web::Data<AppState>,
    job_id: String,
) -> Result<crate::models::EncodingStats, String> {
    use futures::TryStreamExt;

    let start_time = std::time::Instant::now();
    
    // Lire le streaming en chunks
    let mut file_data = Vec::new();
    let mut bytes_received = 0usize;
    
    let mut stream = payload
        .map_err(|e| format!("Erreur de streaming: {}", e))
        .into_async_read();
    
    let mut buffer = [0u8; 8192]; // Buffer de 8KB
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) => break, // Fin du stream
            Ok(n) => {
                file_data.extend_from_slice(&buffer[..n]);
                bytes_received += n;
                
                // Envoyer la progression toutes les 100KB
                if bytes_received % 102400 == 0 {
                    if let Some(ref tx) = data.progress_tx {
                        let progress = (bytes_received as f64 / (bytes_received + 8192) as f64).min(0.9);
                        let _ = tx.send(crate::models::ProgressMessage {
                            job_id: job_id.clone(),
                            progress,
                        });
                    }
                }
            }
            Err(e) => return Err(format!("Erreur de lecture du stream: {}", e)),
        }
    }
    
    if file_data.is_empty() {
        return Err("Aucune donnée reçue".to_string());
    }
    
    // Traiter l'encodage
    let file_size = file_data.len();
    
    // Configurer l'encodeur avec des contraintes appropriées pour le streaming
    let config = adn_core::EncoderConfig {
        encoder_type: adn_core::EncoderType::Fountain,
        chunk_size: 32,
        redundancy: 1.5,
        compression_enabled: true,
        compression_type: adn_core::CompressionType::Lz4,
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
    
    // Sauvegarder le fichier FASTA
    save_fasta_file(&sequences, &job_id).await
        .map_err(|e| format!("Erreur de sauvegarde FASTA: {}", e))?;
    
    // Sauvegarder dans la base de données si activé
    if let Some(db) = &data.database {
        let pool = db.pool().unwrap();
        let repo = adn_storage::SequenceRepository::new(std::sync::Arc::new(pool.clone()));

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
        file_size,
        encoded_size: total_length,
        compression_ratio,
        encoding_time_ms: encoding_time,
    })
}