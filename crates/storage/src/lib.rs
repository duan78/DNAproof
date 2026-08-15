//! Gestion du stockage virtuel ADN

pub mod database;
pub mod error;
pub mod index;
pub mod pool;
pub mod repository;

pub use database::{DatabaseConfig, DatabaseManager, DatabasePool, DatabaseType};
pub use error::{Result, StorageError};
pub use index::{SearchResult, SequenceIndex};
pub use pool::{DnaPool, PoolConfig};
pub use repository::{DbSequence, SequenceRepository};
