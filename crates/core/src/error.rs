//! Types d'erreurs de la bibliothèque.
//!
//! Chaque variante correspond à un échec que l'appelant doit pouvoir distinguer. Une erreur
//! [`DnaError::Integrity`] ou [`DnaError::InsufficientData`] ne doit jamais être convertie en
//! succès : le fichier n'est pas rendu.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DnaError {
    /// Une séquence contient des caractères qui ne sont pas A, C, G ou T.
    #[error("séquence ADN invalide: {0}")]
    InvalidDna(String),

    /// Une séquence ne respecte pas les contraintes biochimiques demandées.
    #[error("contrainte ADN non respectée: {0}")]
    ConstraintViolation(String),

    /// Les lectures reçues ne permettent pas de reconstituer le fichier.
    #[error("données insuffisantes: {0}")]
    InsufficientData(String),

    /// Les données reconstituées ne correspondent pas à l'empreinte embarquée.
    #[error("échec de la vérification d'intégrité: {0}")]
    Integrity(String),

    /// Les données ne sont pas dans un format reconnu ou pris en charge.
    #[error("format non pris en charge: {0}")]
    UnsupportedFormat(String),

    #[error("erreur d'encodage: {0}")]
    Encoding(String),

    #[error("erreur de décodage: {0}")]
    Decoding(String),

    #[error("erreur de correction: {0}")]
    Correction(String),

    #[error("configuration invalide: {0}")]
    InvalidConfig(String),

    #[error("erreur de compression: {0}")]
    Compression(String),

    #[error("erreur d'entrée/sortie: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DnaError>;
