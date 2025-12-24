//! ADN Core Library
//!
//! Bibliothèque principale pour l'encodage/décodage de fichiers en ADN virtuel.

pub mod bio;
pub mod codec;
pub mod constraints;
pub mod error;
pub mod sequence;

// Réexportations principales
pub use error::{DnaError, Result};
pub use sequence::{DnaSequence, DnaConstraints, IupacBase, SequenceId, SequenceMetadata};
pub use codec::{Encoder, Decoder, EncoderConfig, DecoderConfig, ReedSolomonCodec};
pub use constraints::{ConstraintChecker, DnaConstraintValidator};
pub use bio::{IlluminaBarcode, IlluminaAdapter, IlluminaSystem, IlluminaConfig, IlluminaValidator, AdapterType, BarcodePosition};
