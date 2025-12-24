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
pub use constraints::{ConstraintChecker, DnaConstraintValidator};
pub use bio::{IlluminaBarcode, IlluminaAdapter, IlluminaSystem, IlluminaConfig, IlluminaValidator, AdapterType, BarcodePosition};
pub use logging::{init_logging, log_operation, log_error};
pub use performance::{PerformanceCache, PerformanceOptimizer};
