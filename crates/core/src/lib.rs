//! ADN Core Library
//!
//! Bibliothèque principale pour l'encodage/décodage de fichiers en ADN virtuel.

pub mod bio;
pub mod codec;
pub mod constraints;
pub mod error;
pub mod sequence;
pub mod logging;
pub mod performance;

// Réexportations principales
pub use error::{DnaError, Result};
pub use sequence::{DnaSequence, DnaConstraints, IupacBase, SequenceId, SequenceMetadata};
pub use codec::{Encoder, Decoder, EncoderConfig, DecoderConfig, ReedSolomonCodec};
pub use constraints::{ConstraintChecker, DnaConstraintValidator, IncrementalConstraintValidator, IncrementalStats};
pub use bio::{IlluminaBarcode, IlluminaAdapter, IlluminaSystem, IlluminaConfig, IlluminaValidator, AdapterType, BarcodePosition};
pub use logging::init_logging;
// Les macros log_operation et log_error sont automatiquement exportées à la racine du crate
pub use performance::{PerformanceCache, PerformanceOptimizer, HybridCache, AdvancedCacheManager, CacheStrategy};
