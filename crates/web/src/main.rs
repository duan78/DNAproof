//! ADN Storage Web Server
//!
//! Serveur web pour l'encodage/décodage de fichiers en ADN virtuel

use actix_files::Files;
use actix_web::{web, App, HttpServer};
use actix_cors::Cors;
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
            eprintln!("Erreur de chargement de la configuration: {}. Utilisation des valeurs par défaut.", e);
            AppConfig::default()
        }
    };

    // Initialiser le logging
    init_logging(&config.logging);

    // Initialiser la base de données si activée
    let database = if config.database.enabled {
        let mut db_manager = adn_storage::DatabaseManager::new(
            adn_storage::DatabaseConfig {
                db_type: adn_storage::DatabaseType::Sqlite,
                connection_string: config.database.url.clone(),
                max_connections: config.database.max_connections,
            }
        );
        
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

    // Créer l'état de l'application
    let app_state = web::Data::new(AppState {
        tera,
        jobs: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        config: config.clone(),
        database,
    });

    // Configurer CORS
    let cors = Cors::default()
        .allow_any_origin()
        .allow_any_method()
        .allow_any_header()
        .max_age(3600);

    tracing::info!("🧬 Démarrage du serveur ADN Storage sur http://{}:{}", 
        config.server.host, config.server.port);

    HttpServer::new(move || {
        App::new()
            .wrap(TracingLogger::default())
            .wrap(cors.clone())
            .app_data(app_state.clone())
            .service(routes::index)
            .service(routes::encode_page)
            .service(routes::decode_page)
            .service(routes::api_encode)
            .service(routes::api_decode)
            .service(routes::job_status)
            .service(routes::download_result)
            .service(routes::health_check)
            .service(Files::new("/static", &config.server.static_files)
                .show_files_listing())
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


