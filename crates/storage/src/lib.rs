//! Conservation des oligos en base de données (SQLite ou PostgreSQL).
//!
//! Le stockage est annexe à l'archive : celle-ci tient entièrement dans son fichier FASTA. La
//! base sert au serveur web à garder trace des oligos qu'il a produit, par archive.

pub mod database;
pub mod error;
pub mod repository;

pub use database::{
    Argument, DatabaseConfig, DatabaseManager, DatabasePool, DatabaseType, Transaction,
};
pub use error::{Result, StorageError};
pub use repository::{Archive, ArchiveInfo, Inventory, SequenceRepository, StoredOligo};
