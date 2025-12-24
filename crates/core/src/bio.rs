//! Module de bioinformatique pour standards ADN
//!
//! Ce module contient les structures et fonctions pour les standards
//! de séquencement Illumina et autres formats biologiques courants.

pub mod illumina;

pub use illumina::{
    IlluminaBarcode, IlluminaAdapter, AdapterType, IlluminaSystem,
    IlluminaConfig, IlluminaValidator, BarcodePosition,
};
