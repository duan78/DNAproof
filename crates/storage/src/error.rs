//! Gestion des erreurs pour le module de stockage

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Erreur de base de données: {0}")]
    DatabaseError(String),

    #[error("Erreur de configuration: {0}")]
    ConfigError(String),

    #[error("Séquence non trouvée: {0}")]
    SequenceNotFound(String),

    #[error("Erreur d'indexation: {0}")]
    IndexError(String),

    #[error("Erreur de connexion: {0}")]
    ConnectionError(String),

    #[error("Erreur de migration: {0}")]
    MigrationError(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

impl From<sqlx::Error> for StorageError {
    fn from(err: sqlx::Error) -> Self {
        StorageError::DatabaseError(err.to_string())
    }
}

impl From<anyhow::Error> for StorageError {
    fn from(err: anyhow::Error) -> Self {
        StorageError::DatabaseError(err.to_string())
    }
}