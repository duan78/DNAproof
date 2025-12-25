//! Configuration du serveur web

use serde::Deserialize;
use std::path::PathBuf;

/// Configuration du serveur
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub _upload_limit: usize,
    pub static_files: PathBuf,
    pub templates: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            workers: 4,
            _upload_limit: 100 * 1024 * 1024, // 100MB
            static_files: PathBuf::from("./static"),
            templates: PathBuf::from("./templates"),
        }
    }
}

/// Configuration de la base de données
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub enabled: bool,
    pub url: String,
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: ":memory:".to_string(),
            max_connections: 5,
        }
    }
}

/// Configuration complète de l'application
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
}

/// Configuration du logging
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "compact".to_string(),
        }
    }
}

impl AppConfig {
    /// Charge la configuration depuis un fichier
    pub fn load_from_file(path: &str) -> Result<Self, config::ConfigError> {
        let settings = config::Config::builder()
            .add_source(config::File::with_name(path))
            .build()?;

        // Essayer de charger la configuration, sinon utiliser les valeurs par défaut
        let config = settings.try_deserialize::<AppConfig>();

        match config {
            Ok(cfg) => Ok(cfg),
            Err(_) => Ok(AppConfig::default()),
        }
    }
}