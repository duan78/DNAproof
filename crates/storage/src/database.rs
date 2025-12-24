//! Module de base de données pour le stockage ADN

use sqlx::{SqlitePool, PostgresPool, Pool, Sqlite, Postgres};
use std::path::Path;
use async_trait::async_trait;
use tracing::{info, error, instrument};

/// Type de base de données supporté
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseType {
    Sqlite,
    Postgres,
}

/// Configuration de la base de données
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub db_type: DatabaseType,
    pub connection_string: String,
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: DatabaseType::Sqlite,
            connection_string: ":memory:".to_string(),
            max_connections: 5,
        }
    }
}

/// Trait pour les opérations de base de données
#[async_trait]
pub trait DatabaseOperations: Send + Sync {
    async fn initialize(&self) -> crate::Result<()>;
    async fn migrate(&self) -> crate::Result<()>;
    async fn health_check(&self) -> crate::Result<()>;
}

/// Gestionnaire de base de données principal
pub struct DatabaseManager {
    config: DatabaseConfig,
    pool: Option<DatabasePool>,
}

impl DatabaseManager {
    /// Crée un nouveau gestionnaire de base de données
    pub fn new(config: DatabaseConfig) -> Self {
        Self {
            config,
            pool: None,
        }
    }

    /// Connecte à la base de données
    #[instrument(skip(self))]
    pub async fn connect(&mut self) -> crate::Result<()> {
        info!("Connexion à la base de données {}...", 
            match self.config.db_type {
                DatabaseType::Sqlite => "SQLite",
                DatabaseType::Postgres => "PostgreSQL",
            }
        );

        let pool = match self.config.db_type {
            DatabaseType::Sqlite => {
                DatabasePool::Sqlite(
                    SqlitePool::connect(&self.config.connection_string).await?
                )
            }
            DatabaseType::Postgres => {
                DatabasePool::Postgres(
                    PostgresPool::connect(&self.config.connection_string).await?
                )
            }
        };

        self.pool = Some(pool);
        info!("Connexion établie avec succès");
        Ok(())
    }

    /// Retourne le pool de connexions
    pub fn pool(&self) -> crate::Result<&DatabasePool> {
        self.pool.as_ref()
            .ok_or_else(|| crate::StorageError::ConnectionError(
                "Base de données non connectée".to_string()
            ))
    }

    /// Initialise la base de données
    #[instrument(skip(self))]
    pub async fn initialize(&mut self) -> crate::Result<()> {
        self.connect().await?;
        self.migrate().await?;
        Ok(())
    }

    /// Exécute les migrations
    #[instrument(skip(self))]
    pub async fn migrate(&self) -> crate::Result<()> {
        let pool = self.pool()?;
        
        match pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::migrate!("./migrations/sqlite").run(pool).await?;
            }
            DatabasePool::Postgres(pool) => {
                sqlx::migrate!("./migrations/postgres").run(pool).await?;
            }
        }

        info!("Migrations exécutées avec succès");
        Ok(())
    }

    /// Vérifie l'état de santé de la base de données
    #[instrument(skip(self))]
    pub async fn health_check(&self) -> crate::Result<()> {
        let pool = self.pool()?;
        
        match pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
            DatabasePool::Postgres(pool) => {
                sqlx::query("SELECT 1").execute(pool).await?;
            }
        }

        Ok(())
    }
}

/// Énumération des pools de base de données supportés
pub enum DatabasePool {
    Sqlite(Pool<Sqlite>),
    Postgres(Pool<Postgres>),
}

impl DatabasePool {
    /// Exécute une requête SQL générique
    pub async fn execute(&self, query: &str) -> crate::Result<sqlx::query::QueryResult> {
        match self {
            DatabasePool::Sqlite(pool) => {
                Ok(sqlx::query(query).execute(pool).await?)
            }
            DatabasePool::Postgres(pool) => {
                Ok(sqlx::query(query).execute(pool).await?)
            }
        }
    }

    /// Exécute une requête SQL avec retour de résultats
    pub async fn fetch_all(&self, query: &str) -> crate::Result<Vec<sqlx::sqlite::SqliteRow>> {
        match self {
            DatabasePool::Sqlite(pool) => {
                Ok(sqlx::query(query).fetch_all(pool).await?)
            }
            DatabasePool::Postgres(pool) => {
                // Conversion pour PostgreSQL
                let rows = sqlx::query(query).fetch_all(pool).await?;
                // Note: Cela nécessite une conversion appropriée
                unimplemented!("Conversion PostgreSQL vers SqliteRow non implémentée");
            }
        }
    }
}