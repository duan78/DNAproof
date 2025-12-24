//! Gestion du stockage virtuel ADN

pub mod pool;
pub mod index;
pub mod error;
pub mod database;
pub mod repository;

pub use pool::{DnaPool, PoolConfig};
pub use index::{SequenceIndex, SearchResult};
pub use error::{StorageError, Result};
pub use database::{DatabaseManager, DatabaseConfig, DatabaseType, DatabasePool};
pub use repository::{SequenceRepository, DbSequence};
