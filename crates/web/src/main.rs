//! ADN Storage Web Server
//!
//! Serveur web pour l'encodage/décodage de fichiers en ADN virtuel

use actix_cors::Cors;
use actix_files::Files;
use actix_web::{web, App, HttpServer};
use tracing_actix_web::TracingLogger;

mod config;
mod models;
mod routes;

use config::AppConfig;
use models::AppState;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Charger la configuration
    let config = match AppConfig::load_from_file("config.toml") {
        Ok(config) => config,
        Err(e) => {
            eprintln!(
                "Erreur de chargement de la configuration: {}. Utilisation des valeurs par défaut.",
                e
            );
            AppConfig::default()
        }
    };

    // Initialiser le logging
    init_logging(&config.logging);

    // Initialiser la base de données si activée
    let database = if config.database.enabled {
        let mut db_manager = adn_storage::DatabaseManager::new(adn_storage::DatabaseConfig {
            db_type: adn_storage::DatabaseType::Sqlite,
            connection_string: config.database.url.clone(),
            max_connections: config.database.max_connections,
        });

        if let Err(e) = db_manager.initialize().await {
            eprintln!("Erreur d'initialisation de la base de données: {}", e);
            None
        } else {
            Some(db_manager)
        }
    } else {
        None
    };

    // Initialiser Tera
    let tera = match tera::Tera::new(&format!("{}/*", config.server.templates.display())) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Erreur d'initialisation de Tera: {}", e);
            std::process::exit(1);
        }
    };

    // Créer le canal de progression pour les mises à jour temps réel
    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<models::ProgressMessage>();

    // Spawn task to handle progress updates (rate-limited to avoid contention)
    let jobs_for_progress: std::sync::Arc<
        tokio::sync::RwLock<std::collections::HashMap<String, models::JobState>>,
    > = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let jobs_clone = jobs_for_progress.clone();

    tokio::spawn(async move {
        use tokio::time::{interval, Duration};
        let mut ticker = interval(Duration::from_millis(100)); // Max 10 updates/second

        let mut pending_updates: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();

        loop {
            tokio::select! {
                // Receive progress messages
                Some(msg) = progress_rx.recv() => {
                    pending_updates.insert(msg.job_id, msg.progress);
                }
                // Periodically flush updates to job state
                _ = ticker.tick() => {
                    if !pending_updates.is_empty() {
                        let mut jobs = jobs_clone.write().await;
                        for (job_id, progress) in pending_updates.drain() {
                            if let Some(job) = jobs.get_mut(&job_id) {
                                job.progress = Some(progress);
                                job.updated_at = chrono::Utc::now();
                            }
                        }
                    }
                }
            }
        }
    });

    // Créer l'état de l'application
    let app_state = web::Data::new(AppState {
        tera: std::sync::Arc::new(tera),
        jobs: jobs_for_progress,
        config: config.clone(),
        database: database.map(std::sync::Arc::new),
        progress_tx: Some(progress_tx),
    });

    tracing::info!(
        "🧬 Démarrage du serveur ADN Storage sur http://{}:{}",
        config.server.host,
        config.server.port
    );

    HttpServer::new(move || {
        // Configurer CORS (recrée pour chaque worker)
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(TracingLogger::default())
            .wrap(cors)
            .app_data(app_state.clone())
            .service(routes::index)
            .service(routes::encode_page)
            .service(routes::decode_page)
            .service(routes::api_encode)
            .service(routes::api_encode_stream)
            .service(routes::api_decode)
            .service(routes::job_status)
            .service(routes::download_result)
            .service(routes::download_fasta)
            .service(routes::health_check)
            // Pas de listing de répertoire : sert uniquement les fichiers
            // statiques référencés par les templates (évite la divulgation
            // du contenu du dossier).
            .service(Files::new("/static", config.server.static_files.clone()))
    })
    .workers(config.server.workers)
    .bind((config.server.host.clone(), config.server.port))?
    .run()
    .await
}

/// Initialise le système de logging
fn init_logging(config: &crate::config::LoggingConfig) {
    let filter = match config.level.to_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "warn" => "warn",
        "error" => "error",
        _ => "info",
    };

    match config.format.to_lowercase().as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .init();
        }
        _ => {
            tracing_subscriber::fmt()
                .compact()
                .with_env_filter(filter)
                .init();
        }
    }
}
