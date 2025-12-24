//! Gestion du stockage virtuel ADN

pub mod pool;
pub mod index;

pub use pool::{DnaPool, PoolConfig};
pub use index::{SequenceIndex, SearchResult};
