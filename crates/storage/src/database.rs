//! Module de base de données pour le stockage ADN

use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Postgres, Sqlite};
use std::str::FromStr;
use tracing::{info, instrument};

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

/// Gestionnaire de base de données principal
pub struct DatabaseManager {
    config: DatabaseConfig,
    pool: Option<DatabasePool>,
}

impl DatabaseManager {
    /// Crée un nouveau gestionnaire de base de données
    pub fn new(config: DatabaseConfig) -> Self {
        Self { config, pool: None }
    }

    /// Connecte à la base de données
    #[instrument(skip(self))]
    pub async fn connect(&mut self) -> crate::Result<()> {
        info!(
            "Connexion à la base de données {}...",
            match self.config.db_type {
                DatabaseType::Sqlite => "SQLite",
                DatabaseType::Postgres => "PostgreSQL",
            }
        );

        let pool = match self.config.db_type {
            DatabaseType::Sqlite => {
                // Normaliser la chaîne de connexion : sqlx exige une URL
                // `sqlite://...` (un chemin nu comme "adn_storage.db" était
                // rejeté) et ne crée pas le fichier sans create_if_missing.
                let is_memory = self.config.connection_string == ":memory:"
                    || self.config.connection_string == "sqlite::memory:";
                let url = if self.config.connection_string.starts_with("sqlite:") {
                    self.config.connection_string.clone()
                } else if is_memory {
                    "sqlite::memory:".to_string()
                } else {
                    format!("sqlite://{}", self.config.connection_string)
                };

                let mut options = SqliteConnectOptions::from_str(&url)?.create_if_missing(true);

                // Une base en mémoire est privée à chaque connexion : limiter
                // le pool à 1 pour que toutes les requêtes voient les mêmes
                // données (sinon chaque connexion aurait une base vide).
                let max_connections = if is_memory {
                    1
                } else {
                    self.config.max_connections
                };
                if is_memory {
                    options = options.shared_cache(true);
                }

                let sqlite_pool = SqlitePoolOptions::new()
                    .max_connections(max_connections)
                    .connect_with(options)
                    .await?;
                DatabasePool::Sqlite(sqlite_pool)
            }
            DatabaseType::Postgres => {
                let pg_pool = PgPoolOptions::new()
                    .max_connections(self.config.max_connections)
                    .connect(&self.config.connection_string)
                    .await?;
                DatabasePool::Postgres(pg_pool)
            }
        };

        self.pool = Some(pool);
        info!("Connexion établie avec succès");
        Ok(())
    }

    /// Retourne le pool de connexions
    pub fn pool(&self) -> crate::Result<&DatabasePool> {
        self.pool.as_ref().ok_or_else(|| {
            crate::StorageError::ConnectionError("Base de données non connectée".to_string())
        })
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
                sqlx::migrate!("./migrations/sqlite")
                    .run(pool)
                    .await
                    .map_err(|e| crate::StorageError::MigrationError(e.to_string()))?;
            }
            DatabasePool::Postgres(pool) => {
                sqlx::migrate!("./migrations/postgres")
                    .run(pool)
                    .await
                    .map_err(|e| crate::StorageError::MigrationError(e.to_string()))?;
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
/// Pool de connexions à la base de données
#[derive(Clone)]
pub enum DatabasePool {
    Sqlite(Pool<Sqlite>),
    Postgres(Pool<Postgres>),
}

impl DatabasePool {
    /// Exécute une requête SQL générique
    pub async fn execute(&self, query: &str) -> crate::Result<u64> {
        match self {
            DatabasePool::Sqlite(pool) => {
                let result = sqlx::query(query).execute(pool).await?;
                Ok(result.rows_affected())
            }
            DatabasePool::Postgres(pool) => {
                let result = sqlx::query(query).execute(pool).await?;
                Ok(result.rows_affected())
            }
        }
    }

    /// Exécute une requête SQL et retourne le nombre de lignes retournées.
    ///
    /// Note: `execute()` sur un SELECT rapporte toujours 0 ligne affectée —
    /// il faut matérialiser les lignes et les compter.
    pub async fn fetch_count(&self, query: &str) -> crate::Result<u64> {
        match self {
            DatabasePool::Sqlite(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                Ok(rows.len() as u64)
            }
            DatabasePool::Postgres(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                Ok(rows.len() as u64)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sqlite_file_connection_and_crud() {
        // Régression : `Pool::<Sqlite>::connect("chemin_nu")` échouait —
        // sqlx exige une URL sqlite:// et create_if_missing pour créer le
        // fichier. On vérifie connexion + migration + CRUD complet.
        let dir = std::env::temp_dir();
        let db_path = dir.join(format!("adn_storage_test_{}.db", std::process::id()));
        let db_path_str = db_path.to_string_lossy().to_string();

        let mut manager = DatabaseManager::new(DatabaseConfig {
            db_type: DatabaseType::Sqlite,
            connection_string: db_path_str.clone(),
            max_connections: 2,
        });

        manager
            .initialize()
            .await
            .expect("connexion + migrations doivent réussir");

        let pool = match manager.pool().unwrap() {
            DatabasePool::Sqlite(p) => p.clone(),
            _ => panic!("attendait un pool SQLite"),
        };

        let repo = crate::SequenceRepository::new(std::sync::Arc::new(DatabasePool::Sqlite(pool)));

        let bases = vec![
            adn_core::IupacBase::A,
            adn_core::IupacBase::C,
            adn_core::IupacBase::G,
            adn_core::IupacBase::T,
        ];
        let seq = adn_core::DnaSequence::new(bases, "test.bin".to_string(), 0, 4, 42);

        let id = repo.save_sequence(&seq).await.unwrap();
        let fetched = repo
            .get_sequence(id)
            .await
            .unwrap()
            .expect("séquence doit exister");
        assert_eq!(fetched.bases, seq.bases);
        assert_eq!(repo.count_sequences().await.unwrap(), 1);

        // fetch_count doit compter les lignes d'un SELECT
        let count = DatabasePool::Sqlite(match manager.pool().unwrap() {
            DatabasePool::Sqlite(p) => p.clone(),
            _ => unreachable!(),
        })
        .fetch_count("SELECT * FROM sequences")
        .await
        .unwrap();
        assert_eq!(count, 1);

        assert!(repo.delete_sequence(id).await.unwrap());
        assert_eq!(repo.count_sequences().await.unwrap(), 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn test_sqlite_memory_pool() {
        // :memory: avec un pool > 1 : chaque connexion aurait sa propre base
        // vide — le manager limite à 1 connexion dans ce cas.
        let mut manager = DatabaseManager::new(DatabaseConfig {
            db_type: DatabaseType::Sqlite,
            connection_string: ":memory:".to_string(),
            max_connections: 5,
        });
        manager
            .initialize()
            .await
            .expect(":memory: doit se connecter");

        let pool = match manager.pool().unwrap() {
            DatabasePool::Sqlite(p) => p.clone(),
            _ => panic!("attendait un pool SQLite"),
        };

        let repo = crate::SequenceRepository::new(std::sync::Arc::new(DatabasePool::Sqlite(pool)));
        let seq = adn_core::DnaSequence::new(
            vec![adn_core::IupacBase::A, adn_core::IupacBase::T],
            "mem.bin".to_string(),
            0,
            2,
            1,
        );
        let id = repo.save_sequence(&seq).await.unwrap();
        // La lecture doit voir l'écriture (même connexion partagée)
        assert!(repo.get_sequence(id).await.unwrap().is_some());
    }
}
