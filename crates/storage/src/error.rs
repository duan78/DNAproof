//! Erreurs du module de stockage.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("erreur de base de données : {0}")]
    DatabaseError(String),

    #[error("erreur de configuration : {0}")]
    ConfigError(String),

    #[error("erreur de connexion : {0}")]
    ConnectionError(String),

    #[error("erreur de migration : {0}")]
    MigrationError(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        StorageError::DatabaseError(err.to_string())
    }
}

impl From<sqlx::migrate::MigrateError> for StorageError {
    fn from(err: sqlx::migrate::MigrateError) -> Self {
        StorageError::MigrationError(err.to_string())
    }
}
