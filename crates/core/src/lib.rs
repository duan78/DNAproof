//! ADN Core Library
//!
//! Bibliothèque principale pour l'encodage/décodage de fichiers en ADN virtuel.

pub mod bio;
pub mod codec;
pub mod constraints;
pub mod error;
pub mod logging;
pub mod performance;
pub mod sequence;

// Réexportations principales
pub use bio::{
    AdapterType, BarcodePosition, IlluminaAdapter, IlluminaBarcode, IlluminaConfig, IlluminaSystem,
    IlluminaValidator,
};
pub use codec::{Decoder, DecoderConfig, Encoder, EncoderConfig, EncoderType, ReedSolomonCodec};
pub use constraints::{
    ConstraintChecker, DnaConstraintValidator, IncrementalConstraintValidator, IncrementalStats,
};
pub use error::{DnaError, Result};
pub use logging::init_logging;
pub use sequence::{DnaConstraints, DnaSequence, IupacBase, SequenceId, SequenceMetadata};
// Les macros log_operation et log_error sont automatiquement exportées à la racine du crate
pub use performance::{
    AdvancedCacheManager, CacheStrategy, HybridCache, PerformanceCache, PerformanceOptimizer,
};
