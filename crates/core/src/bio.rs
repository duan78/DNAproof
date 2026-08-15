//! Module de bioinformatique pour standards ADN
//!
//! Ce module contient les structures et fonctions pour les standards
//! de séquencement Illumina et autres formats biologiques courants.

pub mod illumina;

pub use illumina::{
    AdapterType, BarcodePosition, IlluminaAdapter, IlluminaBarcode, IlluminaConfig, IlluminaSystem,
    IlluminaValidator,
};
