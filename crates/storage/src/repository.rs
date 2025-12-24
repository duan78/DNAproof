//! Repository pour les opérations de stockage ADN

use crate::{DatabasePool, Result, StorageError};
use adn_core::{DnaSequence, SequenceMetadata};
use sqlx::FromRow;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use tracing::{info, error, instrument};

/// Modèle de séquence ADN pour la base de données
#[derive(Debug, FromRow)]
pub struct DbSequence {
    pub id: i64,
    pub uuid: String,
    pub sequence_data: String,
    pub metadata: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Repository pour les opérations sur les séquences ADN
pub struct SequenceRepository {
    pool: DatabasePool,
}

impl SequenceRepository {
    /// Crée un nouveau repository
    pub fn new(pool: DatabasePool) -> Self {
        Self { pool }
    }

    /// Sauvegarde une séquence ADN
    #[instrument(skip(self, sequence))]
    pub async fn save_sequence(&self, sequence: &DnaSequence) -> Result<i64> {
        let metadata_json = serde_json::to_string(&sequence.metadata)
            .map_err(|e| StorageError::DatabaseError(e.to_string()))?;
        
        let sequence_data = sequence.bases.iter()
            .map(|base| base.to_char())
            .collect::<String>();

        let query = 
            "INSERT INTO sequences (uuid, sequence_data, metadata, created_at, updated_at) 
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id";

        let row = match &self.pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(Uuid::new_v4().to_string())
                    .bind(sequence_data)
                    .bind(metadata_json)
                    .bind(Utc::now())
                    .bind(Utc::now())
                    .fetch_one(pool)
                    .await?
            }
            DatabasePool::Postgres(pool) => {
                sqlx::query(query)
                    .bind(Uuid::new_v4().to_string())
                    .bind(sequence_data)
                    .bind(metadata_json)
                    .bind(Utc::now())
                    .bind(Utc::now())
                    .fetch_one(pool)
                    .await?
            }
        };

        let id: i64 = row.try_get("id")?;
        info!("Séquence sauvegardée avec ID: {}", id);
        Ok(id)
    }

    /// Récupère une séquence par ID
    #[instrument(skip(self))]
    pub async fn get_sequence(&self, id: i64) -> Result<Option<DnaSequence>> {
        let query = "SELECT * FROM sequences WHERE id = $1";

        let row = match &self.pool {
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

        if let Some(db_seq) = row {
            self.db_sequence_to_dna_sequence(db_seq).await
        } else {
            Ok(None)
        }
    }

    /// Recherche des séquences par métadonnées
    #[instrument(skip(self))]
    pub async fn search_sequences(&self, query: &str) -> Result<Vec<DnaSequence>> {
        let search_query = 
            "SELECT * FROM sequences 
             WHERE metadata LIKE $1 
             ORDER BY created_at DESC";

        let rows = match &self.pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::query_as::<_, DbSequence>(search_query)
                    .bind(format!("%{}", query))
                    .fetch_all(pool)
                    .await?
            }
            DatabasePool::Postgres(pool) => {
                sqlx::query_as::<_, DbSequence>(search_query)
                    .bind(format!("%{}", query))
                    .fetch_all(pool)
                    .await?
            }
        };

        let mut sequences = Vec::new();
        for db_seq in rows {
            if let Some(seq) = self.db_sequence_to_dna_sequence(db_seq).await? {
                sequences.push(seq);
            }
        }

        Ok(sequences)
    }

    /// Supprime une séquence
    #[instrument(skip(self))]
    pub async fn delete_sequence(&self, id: i64) -> Result<bool> {
        let query = "DELETE FROM sequences WHERE id = $1";

        let result = match &self.pool {
            DatabasePool::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(id)
                    .execute(pool)
                    .await?
            }
            DatabasePool::Postgres(pool) => {
                sqlx::query(query)
                    .bind(id)
                    .execute(pool)
                    .await?
            }
        };

        Ok(result.rows_affected() > 0)
    }

    /// Convertit une DbSequence en DnaSequence
    async fn db_sequence_to_dna_sequence(&self, db_seq: DbSequence) -> Result<Option<DnaSequence>> {
        let metadata: SequenceMetadata = serde_json::from_str(&db_seq.metadata)
            .map_err(|e| StorageError::DatabaseError(e.to_string()))?;

        let bases = db_seq.sequence_data.chars()
            .filter_map(|c| match c {
                'A' => Some(adn_core::IupacBase::A),
                'C' => Some(adn_core::IupacBase::C),
                'G' => Some(adn_core::IupacBase::G),
                'T' => Some(adn_core::IupacBase::T),
                _ => None,
            })
            .collect();

        Ok(Some(DnaSequence::new(
            bases,
            metadata.sequence_type,
            metadata.chunk_index,
            metadata.original_length,
            metadata.seed,
        )))
    }
}