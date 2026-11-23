//! Configuration du serveur : un fichier TOML, surchargé par des variables d'environnement.

use adn_storage::DatabaseType;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Taille maximale des fichiers acceptés, par défaut : 25 Mio. Un fichier incompressible donne
/// environ huit fois sa taille d'ADN, en mémoire pendant l'encodage puis sur disque.
pub const DEFAULT_UPLOAD_LIMIT: usize = 25 * 1024 * 1024;

/// Serveur HTTP.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    /// Noms d'hôtes acceptés en plus de la boucle locale, sans port — par exemple le nom public
    /// d'un proxy inverse qui préserve l'en-tête `Host`. Vide : seuls les hôtes de la boucle
    /// locale sont acceptés quand le serveur y est lié ; tous le sont sinon.
    pub allowed_hosts: Vec<String>,
    pub port: u16,
    /// Nombre de threads qui servent les requêtes.
    pub workers: usize,
    /// Taille maximale des fichiers acceptés, en octets.
    pub upload_limit: usize,
    /// Nombre d'encodages et de décodages menés de front.
    pub max_concurrent_jobs: usize,
    /// Durée de conservation des résultats, en minutes ; `0` pour ne jamais les effacer.
    pub job_retention_minutes: u64,
    /// Dossier où sont écrits les résultats.
    pub storage_dir: PathBuf,
    pub static_files: PathBuf,
    pub templates: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            allowed_hosts: Vec::new(),
            port: 8080,
            workers: 2,
            upload_limit: DEFAULT_UPLOAD_LIMIT,
            max_concurrent_jobs: 2,
            job_retention_minutes: 24 * 60,
            storage_dir: PathBuf::from("./uploads"),
            static_files: PathBuf::from("./static"),
            templates: PathBuf::from("./templates"),
        }
    }
}

/// Base de données : un miroir des archives, facultatif.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    pub enabled: bool,
    /// `sqlite` ou `postgres`.
    #[serde(rename = "type")]
    pub kind: DatabaseType,
    /// Chemin d'un fichier SQLite, ou URL PostgreSQL.
    pub url: String,
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: DatabaseType::Sqlite,
            url: "adn_storage.db".to_string(),
            max_connections: 5,
        }
    }
}

impl DatabaseConfig {
    /// Paramètres de connexion attendus par la couche de stockage.
    pub fn connection(&self) -> adn_storage::DatabaseConfig {
        let mut config = match self.kind {
            DatabaseType::Sqlite => adn_storage::DatabaseConfig::sqlite(self.url.clone()),
            DatabaseType::Postgres => adn_storage::DatabaseConfig::postgres(self.url.clone()),
        };
        config.max_connections = self.max_connections;
        config
    }
}

/// Journaux.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: String,
    /// `compact` ou `json`.
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

/// Configuration complète.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
}

impl AppConfig {
    /// Lit le fichier `path` s'il existe, puis applique les variables d'environnement
    /// `ADN_<SECTION>__<CLÉ>` (par exemple `ADN_SERVER__PORT=9000`).
    ///
    /// Une clé inconnue, dans le fichier ou l'environnement, est une erreur : le serveur ne doit
    /// pas démarrer avec un réglage ignoré en silence.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let name = path
            .to_str()
            .ok_or_else(|| ConfigError("chemin de configuration illisible".to_string()))?;
        let settings = config::Config::builder()
            .add_source(config::File::with_name(name).required(false))
            .add_source(
                config::Environment::with_prefix("ADN")
                    .prefix_separator("_")
                    .separator("__")
                    // Les valeurs restent du texte : une URL ou un mot de passe qui ressemble à
                    // un nombre doit rester une chaîne.
                    .try_parsing(false),
            )
            .build()
            .map_err(|error| ConfigError(error.to_string()))?;
        let config: AppConfig = settings
            .try_deserialize()
            .map_err(|error| ConfigError(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Relit une configuration donnée en TOML, sans fichier ni environnement.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let settings = config::Config::builder()
            .add_source(config::File::from_str(text, config::FileFormat::Toml))
            .build()
            .map_err(|error| ConfigError(error.to_string()))?;
        let config: AppConfig = settings
            .try_deserialize()
            .map_err(|error| ConfigError(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Vérifie que les réglages sont applicables.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let server = &self.server;
        if server.host.trim().is_empty() {
            return Err(ConfigError("server.host ne peut pas être vide".to_string()));
        }
        if server.workers == 0 {
            return Err(ConfigError(
                "server.workers doit valoir au moins 1".to_string(),
            ));
        }
        if let Some(empty) = server
            .allowed_hosts
            .iter()
            .find(|host| host.trim().is_empty())
        {
            return Err(ConfigError(format!(
                "server.allowed_hosts contient un nom vide ({empty:?})"
            )));
        }
        if server.upload_limit == 0 {
            return Err(ConfigError(
                "server.upload_limit doit valoir au moins 1 octet".to_string(),
            ));
        }
        if server.max_concurrent_jobs == 0 {
            return Err(ConfigError(
                "server.max_concurrent_jobs doit valoir au moins 1".to_string(),
            ));
        }
        if !matches!(
            self.logging.format.to_ascii_lowercase().as_str(),
            "compact" | "json"
        ) {
            return Err(ConfigError(
                "logging.format doit valoir « compact » ou « json »".to_string(),
            ));
        }
        if self.database.enabled {
            self.database.connection().validate()?;
        }
        Ok(())
    }

    /// Niveau de journal le plus verbose accepté par `tracing`.
    pub fn log_filter(&self) -> String {
        self.logging.level.to_ascii_lowercase()
    }
}

/// Configuration inapplicable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl From<adn_storage::StorageError> for ConfigError {
    fn from(error: adn_storage::StorageError) -> Self {
        ConfigError(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_apply_to_an_empty_file() {
        let config = AppConfig::from_toml("").unwrap();
        assert_eq!(config, AppConfig::default());
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.upload_limit, DEFAULT_UPLOAD_LIMIT);
        assert!(!config.database.enabled);
    }

    #[test]
    fn every_setting_is_read() {
        let config = AppConfig::from_toml(
            r#"
[server]
host = "0.0.0.0"
port = 9000
workers = 3
upload_limit = 1048576
max_concurrent_jobs = 4
job_retention_minutes = 0
storage_dir = "/var/adn"
allowed_hosts = ["adn.example.org"]

[database]
enabled = true
type = "postgres"
url = "postgres://adn:1234@db/adn"
max_connections = 10

[logging]
level = "debug"
format = "json"
"#,
        )
        .unwrap();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 9000);
        assert_eq!(config.server.workers, 3);
        assert_eq!(config.server.upload_limit, 1048576);
        assert_eq!(config.server.max_concurrent_jobs, 4);
        assert_eq!(config.server.job_retention_minutes, 0);
        assert_eq!(
            config.server.allowed_hosts,
            vec!["adn.example.org".to_string()]
        );
        assert_eq!(config.server.static_files, PathBuf::from("./static"));
        assert_eq!(config.database.kind, DatabaseType::Postgres);
        let connection = config.database.connection();
        assert_eq!(connection.connection_string, "postgres://adn:1234@db/adn");
        assert_eq!(connection.max_connections, 10);
        assert_eq!(config.logging.format, "json");
    }

    #[test]
    fn partial_settings_keep_the_other_defaults() {
        let config = AppConfig::from_toml("[server]\nport = 8081\n").unwrap();
        assert_eq!(config.server.port, 8081);
        assert_eq!(config.server.upload_limit, DEFAULT_UPLOAD_LIMIT);
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn unknown_settings_are_refused() {
        // Régression : une clé mal orthographiée faisait ignorer tout le réglage.
        let error = AppConfig::from_toml("[server]\nupload_limit = 10\nupload_limt = 20\n");
        assert!(error.is_err());
        let error = AppConfig::from_toml("[databse]\nenabled = true\n");
        assert!(error.unwrap_err().0.contains("databse"));
    }

    #[test]
    fn inapplicable_settings_are_refused() {
        let invalid = |text: &str| AppConfig::from_toml(text).is_err();
        assert!(invalid("[server]\nworkers = 0\n"));
        assert!(invalid("[server]\nupload_limit = 0\n"));
        assert!(invalid("[server]\nmax_concurrent_jobs = 0\n"));
        assert!(invalid("[server]\nhost = \" \"\n"));
        assert!(invalid("[logging]\nformat = \"xml\"\n"));
        assert!(invalid("[database]\nenabled = true\nurl = \" \"\n"));
        assert!(invalid("[database]\nenabled = true\nmax_connections = 0\n"));
        assert!(invalid("[database]\ntype = \"mysql\"\n"));
        assert!(invalid("[server]\nport = 70000\n"));
        assert!(invalid(
            "[server]\nallowed_hosts = [\"adn.example.org\", \" \"]\n"
        ));
        assert!(AppConfig::from_toml("[server]\njob_retention_minutes = 0\n").is_ok());
    }

    #[test]
    fn a_broken_file_stops_the_server_but_a_missing_one_does_not() {
        let directory = std::env::temp_dir().join(format!("adn-web-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let absent = AppConfig::load(&directory.join("absent.toml"));

        let broken = directory.join("broken.toml");
        std::fs::write(&broken, "[server\nport = 1\n").unwrap();
        let refused = AppConfig::load(&broken);
        std::fs::remove_dir_all(&directory).unwrap();

        assert!(
            absent.is_ok(),
            "sans fichier, les valeurs par défaut s'appliquent"
        );
        assert!(
            refused.is_err(),
            "un fichier illisible doit arrêter le démarrage"
        );
    }
}
