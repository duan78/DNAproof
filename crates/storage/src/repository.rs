//! Repository pour les opérations de stockage ADN

use crate::{DatabasePool, Result, StorageError};
use adn_core::{DnaSequence, IupacBase};
use sqlx::{FromRow, Row};
use uuid::Uuid;
use tracing::{info, instrument};
use chrono::Utc;

/// Modèle de séquence ADN pour la base de données
#[derive(Debug, FromRow)]
pub struct DbSequence {
    pub id: i64,
    pub uuid: String,
    pub sequence_data: String,
    pub metadata: String,
    pub created_at: String,  // Stocké comme ISO 8601 string
    pub updated_at: String,  // Stocké comme ISO 8601 string
}

/// Repository pour les opérations sur les séquences ADN
pub struct SequenceRepository {
    pool: std::sync::Arc<DatabasePool>,
}

impl SequenceRepository {
    /// Crée un nouveau repository
    pub fn new(pool: std::sync::Arc<DatabasePool>) -> Self {
        Self { pool }
    }

    /// Sauvegarde une séquence ADN
    #[instrument(skip(self, sequence))]
    pub async fn save_sequence(&self, sequence: &DnaSequence) -> Result<i64> {
        let metadata_json = serde_json::to_string(&sequence.metadata)
            .map_err(|e| StorageError::DatabaseError(e.to_string()))?;

        let sequence_data = sequence.bases.iter()
            .map(|base| base.as_char())
            .collect::<String>();

        let now = Utc::now().to_rfc3339();

        let query =
            "INSERT INTO sequences (uuid, sequence_data, metadata, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id";

        let id = match &*self.pool {
            DatabasePool::Sqlite(pool) => {
                let row = sqlx::query(query)
                    .bind(Uuid::new_v4().to_string())
                    .bind(sequence_data)
                    .bind(metadata_json)
                    .bind(&now)
                    .bind(&now)
                    .fetch_one(pool)
                    .await?;
                row.try_get("id")?
            }
            DatabasePool::Postgres(pool) => {
                let row = sqlx::query(query)
                    .bind(Uuid::new_v4().to_string())
                    .bind(sequence_data)
                    .bind(metadata_json)
                    .bind(&now)
                    .bind(&now)
                    .fetch_one(pool)
                    .await?;
                row.try_get("id")?
            }
        };

        info!("Séquence sauvegardée avec ID: {}", id);
        Ok(id)
    }

    /// Récupère une séquence par ID
    #[instrument(skip(self))]
    pub async fn get_sequence(&self, id: i64) -> Result<Option<DnaSequence>> {
        let query = "SELECT * FROM sequences WHERE id = $1";

        let row = match &*self.pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::query_as::<_, DbSequence>(query)
                    .bind(id)
                    .fetch_optional(pool)
                    .await?
            }
            DatabasePool::Postgres(pool) => {
                sqlx::query_as::<_, DbSequence>(query)
                    .bind(id)
                    .fetch_optional(pool)
                    .await?
            }
        };

        match row {
            Some(db_seq) => Ok(Some(self.db_sequence_to_dna_sequence(db_seq)?)),
            None => Ok(None),
        }
    }

    /// Recherche des séquences par métadonnées
    #[instrument(skip(self))]
    pub async fn search_sequences(&self, query_str: &str) -> Result<Vec<DnaSequence>> {
        let search_query =
            "SELECT * FROM sequences
             WHERE metadata LIKE $1
             ORDER BY created_at DESC";

        let rows = match &*self.pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::query_as::<_, DbSequence>(search_query)
                    .bind(format!("%{}", query_str))
                    .fetch_all(pool)
                    .await?
            }
            DatabasePool::Postgres(pool) => {
                sqlx::query_as::<_, DbSequence>(search_query)
                    .bind(format!("%{}", query_str))
                    .fetch_all(pool)
                    .await?
            }
        };

        let mut sequences = Vec::new();
        for row in rows {
            sequences.push(self.db_sequence_to_dna_sequence(row)?);
        }
        Ok(sequences)
    }

    /// Supprime une séquence par ID
    #[instrument(skip(self))]
    pub async fn delete_sequence(&self, id: i64) -> Result<bool> {
        let query = "DELETE FROM sequences WHERE id = $1";

        match &*self.pool {
            DatabasePool::Sqlite(pool) => {
                let result = sqlx::query(query).bind(id).execute(pool).await?;
                Ok(result.rows_affected() > 0)
            }
            DatabasePool::Postgres(pool) => {
                let result = sqlx::query(query).bind(id).execute(pool).await?;
                Ok(result.rows_affected() > 0)
            }
        }
    }

    /// Compte le nombre total de séquences
    #[instrument(skip(self))]
    pub async fn count_sequences(&self) -> Result<i64> {
        let query = "SELECT COUNT(*) as count FROM sequences";

        match &*self.pool {
            DatabasePool::Sqlite(pool) => {
                let row = sqlx::query(query).fetch_one(pool).await?;
                Ok(row.try_get("count")?)
            }
            DatabasePool::Postgres(pool) => {
                let row = sqlx::query(query).fetch_one(pool).await?;
                Ok(row.try_get("count")?)
            }
        }
    }

    /// Convertit DbSequence en DnaSequence
    fn db_sequence_to_dna_sequence(&self, db_seq: DbSequence) -> Result<DnaSequence> {
        use adn_core::SequenceId;

        // Parse les bases
        let bases: Vec<IupacBase> = db_seq.sequence_data
            .chars()
            .map(|c| {
                IupacBase::from_char(c)
                    .map_err(|e| StorageError::DatabaseError(format!("Invalid base: {}", e)))
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| StorageError::DatabaseError(format!("Invalid base sequence: {}", e)))?;

        // Parse les métadonnées
        let metadata = serde_json::from_str(&db_seq.metadata)
            .map_err(|e| StorageError::DatabaseError(format!("Failed to parse metadata: {}", e)))?;

        // Générer un ID de séquence depuis l'UUID string
        let uuid = Uuid::parse_str(&db_seq.uuid)
            .map_err(|e| StorageError::DatabaseError(format!("Invalid UUID: {}", e)))?;

        Ok(DnaSequence {
            id: SequenceId::from_uuid(uuid),
            bases,
            metadata,
        })
    }
}
