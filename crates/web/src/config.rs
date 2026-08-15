//! Configuration du serveur web

use serde::Deserialize;
use std::path::PathBuf;

/// Configuration du serveur
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workers: usize,
    /// Taille maximale des uploads acceptés (octets)
    pub upload_limit: usize,
    pub static_files: PathBuf,
    pub templates: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            workers: 4,
            upload_limit: 100 * 1024 * 1024, // 100MB
            static_files: PathBuf::from("./static"),
            templates: PathBuf::from("./templates"),
        }
    }
}

/// Configuration de la base de données
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
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
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
}

/// Configuration du logging
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
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
    /// Charge la configuration depuis un fichier, avec surcharge par les
    /// variables d'environnement préfixées `ADN_` (séparateur `__` pour les
    /// sous-sections, ex. `ADN_SERVER__HOST=0.0.0.0`).
    ///
    /// Les clés absentes prennent les valeurs par défaut (serde(default)).
    /// Une erreur de désérialisation (type inattendu) est propagée : le
    /// serveur ne doit pas démarrer silencieusement avec une config ignorée.
    pub fn load_from_file(path: &str) -> Result<Self, config::ConfigError> {
        let settings = config::Config::builder()
            // Fichier optionnel : absent, on retombe sur les défauts +
            // variables d'environnement (sans quoi ADN_SERVER__PORT etc.
            // seraient ignorés quand le fichier n'existe pas)
            .add_source(config::File::with_name(path).required(false))
            .add_source(
                config::Environment::with_prefix("ADN")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        settings.try_deserialize::<AppConfig>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_toml_deserializes() {
        // Régression : le champ s'appelait `_upload_limit` côté struct mais
        // `upload_limit` dans config.toml → tout le fichier était ignoré.
        let toml = r#"
[server]
host = "0.0.0.0"
port = 9090
upload_limit = 52428800

[database]
enabled = false

[logging]
level = "debug"
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.port, 9090);
        assert_eq!(cfg.server.upload_limit, 52_428_800);
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.logging.level, "debug");
    }

    #[test]
    fn test_partial_config_uses_defaults() {
        let toml = r#"
[server]
port = 8081
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.port, 8081);
        assert_eq!(
            cfg.server.upload_limit,
            ServerConfig::default().upload_limit
        );
        assert_eq!(cfg.logging.level, "info");
    }
}

#[cfg(test)]
mod env_tests {
    use super::*;

    #[test]
    fn test_env_override() {
        // Les variables d'environnement ADN_* doivent s'appliquer même quand
        // le fichier de config est absent (utile en conteneur Docker).
        std::env::set_var("ADN_SERVER__PORT", "18099");
        let cfg = AppConfig::load_from_file("fichier_inexistant.toml").unwrap();
        assert_eq!(
            cfg.server.port, 18099,
            "ADN_SERVER__PORT doit être appliqué même sans fichier de config"
        );
        std::env::remove_var("ADN_SERVER__PORT");
    }
}
