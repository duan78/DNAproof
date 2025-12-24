//! Types d'erreurs pour la bibliothèque ADN

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DnaError {
    #[error("Violation de contrainte ADN: {0}")]
    ConstraintViolation(String),

    #[error("Données insuffisantes pour la récupération: besoin de {need} gouttes, avons {have}")]
    InsufficientData { need: usize, have: usize },

    #[error("Checksum mismatch: attendu {expected}, obtenu {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Base IUPAC invalide: {0}")]
    InvalidBase(char),

    #[error("Séquence trop longue: {len} > {max}")]
    SequenceTooLong { len: usize, max: usize },

    #[error("Homopolymer run détecté: {base}x{count}")]
    HomopolymerRun { base: char, count: usize },

    #[error("GC content hors plage: {gc:.2} pas dans [{min:.2}, {max:.2}]")]
    GcContentOutOfRange { gc: f64, min: f64, max: f64 },

    #[error("Erreur IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("Erreur de sérialisation: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Erreur d'encodage: {0}")]
    Encoding(String),

    #[error("Erreur de décodage: {0}")]
    Decoding(String),

    #[error("Erreur de correction: {0}")]
    Correction(String),

    #[error("Données corrompues irrécupérables")]
    DataCorrupted,
}

pub type Result<T> = std::result::Result<T, DnaError>;
