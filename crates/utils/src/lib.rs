//! Utilitaires partagés

pub mod conversion;
pub mod math;

pub use conversion::{BytesToDna, DnaToBytes};
pub use math::{entropy, EntropyConfig};
