//! Conservation des archives : oligos rangés par archive, par lots, dans une transaction.

use crate::database::on_pool;
use crate::{Argument, DatabasePool, Result, StorageError};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::fmt::Write as _;
use std::sync::Arc;
use uuid::Uuid;

/// Oligos par requête d'insertion. Sept paramètres par ligne : la limite de SQLite (999
/// paramètres) est respectée avec de la marge.
pub const BATCH_SIZE: usize = 100;

/// Archive à conserver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveInfo {
    /// Nom de l'archive, qui range ses oligos.
    pub id: String,
    /// Nom du fichier d'origine.
    pub file_name: String,
    /// Empreinte SHA-256 du fichier d'origine, en hexadécimal.
    pub sha256: String,
    /// Schéma d'encodage (`fountain` ou `strands`).
    pub scheme: String,
    /// Mesures de l'encodage, en JSON (un objet), rendues telles quelles par l'API.
    pub stats: String,
}

/// Archive conservée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Archive {
    pub info: ArchiveInfo,
    /// Nombre d'oligos rangés.
    pub oligos: i64,
    /// Date de conservation, au format RFC 3339.
    pub created_at: String,
}

/// Bilan d'une base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inventory {
    /// Nombre d'archives.
    pub archives: i64,
    /// Nombre d'oligos, y compris ceux de la version 0.1, qui n'appartiennent à aucune archive.
    pub oligos: i64,
}

/// Oligo conservé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredOligo {
    pub id: i64,
    pub uuid: Uuid,
    pub sequence: Vec<u8>,
    /// Nom de l'archive d'origine.
    pub archive: String,
    /// Rang de l'oligo dans l'archive.
    pub position: usize,
}

/// Contenu de la colonne `metadata`, héritée du format 0.1 : les mêmes informations que les
/// colonnes `archive` et `position`, en JSON.
#[derive(Serialize, Deserialize)]
struct Metadata<'a> {
    archive: &'a str,
    position: usize,
}

/// Accès aux archives.
#[derive(Clone)]
pub struct SequenceRepository {
    pool: Arc<DatabasePool>,
}

impl SequenceRepository {
    /// Travaille sur le pool donné.
    pub fn new(pool: Arc<DatabasePool>) -> Self {
        Self { pool }
    }

    /// Conserve les oligos d'une archive, dans l'ordre. Rend le nombre d'oligos rangés.
    ///
    /// Tout ou rien : la description de l'archive et ses oligos sont écrits dans une seule
    /// transaction. Une archive de même nom est remplacée. Sans oligos, rien n'est écrit.
    pub async fn save_archive(&self, info: &ArchiveInfo, sequences: &[Vec<u8>]) -> Result<u64> {
        for (what, value) in [
            ("le nom de l'archive", info.id.as_str()),
            ("le nom du fichier", info.file_name.as_str()),
            ("l'empreinte", info.sha256.as_str()),
            ("le schéma", info.scheme.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(StorageError::ConfigError(format!(
                    "{what} ne peut pas être vide"
                )));
            }
        }
        if !matches!(
            serde_json::from_str::<serde_json::Value>(&info.stats),
            Ok(serde_json::Value::Object(_))
        ) {
            return Err(StorageError::ConfigError(
                "les mesures de l'archive doivent être un objet JSON".to_string(),
            ));
        }
        // Tout est vérifié avant la première écriture : un oligo refusé ne doit pas laisser
        // une archive à moitié rangée.
        let texts = sequences
            .iter()
            .map(|sequence| {
                std::str::from_utf8(sequence)
                    .map(|text| text.trim().to_string())
                    .map_err(|_| {
                        StorageError::ConfigError(
                            "les séquences doivent être du texte (A, C, G, T)".to_string(),
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        if texts.is_empty() {
            return Ok(0);
        }
        let now = Timestamp::now().to_string();
        let id = info.id.as_str();

        let mut transaction = self.pool.begin().await?;
        transaction
            .execute("DELETE FROM sequences WHERE archive = $1", &[id.into()])
            .await?;
        transaction
            .execute("DELETE FROM archives WHERE id = $1", &[id.into()])
            .await?;
        transaction
            .execute(
                "INSERT INTO archives (id, file_name, sha256, scheme, stats, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    id.into(),
                    info.file_name.as_str().into(),
                    info.sha256.as_str().into(),
                    info.scheme.as_str().into(),
                    info.stats.as_str().into(),
                    now.as_str().into(),
                ],
            )
            .await?;
        for (batch_rank, batch) in texts.chunks(BATCH_SIZE).enumerate() {
            // `$1`…`$7` pour la première ligne, `$8`…`$14` pour la suivante, et ainsi de suite.
            let mut sql = String::with_capacity(170 * batch.len() + 130);
            sql.push_str(
                "INSERT INTO sequences \
                 (uuid, sequence_data, metadata, created_at, updated_at, archive, position) VALUES ",
            );
            let mut arguments: Vec<Argument> = Vec::with_capacity(7 * batch.len());
            for (rank, text) in batch.iter().enumerate() {
                if rank > 0 {
                    sql.push_str(", ");
                }
                let first = 7 * rank + 1;
                write!(
                    sql,
                    "(${first}, ${}, ${}, ${}, ${}, ${}, ${})",
                    first + 1,
                    first + 2,
                    first + 3,
                    first + 4,
                    first + 5,
                    first + 6
                )
                .expect("écrire dans une chaîne en mémoire");

                let position = batch_rank * BATCH_SIZE + rank;
                let metadata = serde_json::to_string(&Metadata {
                    archive: id,
                    position,
                })
                .map_err(|e| StorageError::DatabaseError(e.to_string()))?;
                arguments.push(Uuid::new_v4().to_string().into());
                arguments.push(text.as_str().into());
                arguments.push(metadata.into());
                arguments.push(now.as_str().into());
                arguments.push(now.as_str().into());
                arguments.push(id.into());
                arguments.push((position as i64).into());
            }
            transaction
                .execute(sqlx::AssertSqlSafe(sql), &arguments)
                .await?;
        }
        transaction.commit().await?;
        Ok(texts.len() as u64)
    }

    /// Une archive, par son nom.
    pub async fn archive(&self, id: &str) -> Result<Option<Archive>> {
        Ok(self
            .archives_where("WHERE a.id = $1", &[id.into()])
            .await?
            .into_iter()
            .next())
    }

    /// Les archives, de la plus récente à la plus ancienne.
    pub async fn archives(&self) -> Result<Vec<Archive>> {
        self.archives_where("", &[]).await
    }

    /// Les oligos d'une archive, dans l'ordre.
    pub async fn archive_sequences(&self, archive: &str) -> Result<Vec<Vec<u8>>> {
        on_pool!(self.pool.as_ref(), executor => {
            crate::database::fetch(
                executor,
                "SELECT sequence_data FROM sequences WHERE archive = $1 ORDER BY position, id",
                &[archive.into()],
                |row| -> Result<Vec<u8>> {
                    let sequence: String = row.try_get("sequence_data")?;
                    Ok(sequence.into_bytes())
                },
            )
            .await
        })
    }

    /// Supprime une archive et ses oligos ; rend le nombre d'oligos supprimés.
    pub async fn delete_archive(&self, archive: &str) -> Result<u64> {
        let mut transaction = self.pool.begin().await?;
        let removed = transaction
            .execute(
                "DELETE FROM sequences WHERE archive = $1",
                &[archive.into()],
            )
            .await?;
        transaction
            .execute("DELETE FROM archives WHERE id = $1", &[archive.into()])
            .await?;
        transaction.commit().await?;
        Ok(removed)
    }

    /// Nombre d'oligos conservés.
    pub async fn count(&self) -> Result<i64> {
        on_pool!(self.pool.as_ref(), executor => {
            crate::database::fetch(
                executor,
                "SELECT COUNT(*) AS count FROM sequences",
                &[],
                |row| -> Result<i64> { Ok(row.try_get("count")?) },
            )
            .await
        })
        .map(|counts: Vec<i64>| counts.into_iter().next().unwrap_or(0))
    }

    /// Nombre d'archives et d'oligos conservés.
    pub async fn inventory(&self) -> Result<Inventory> {
        on_pool!(self.pool.as_ref(), executor => {
            crate::database::fetch(
                executor,
                "SELECT (SELECT COUNT(*) FROM archives) AS archives, \
                 (SELECT COUNT(*) FROM sequences) AS oligos",
                &[],
                |row| -> Result<Inventory> {
                    Ok(Inventory {
                        archives: row.try_get("archives")?,
                        oligos: row.try_get("oligos")?,
                    })
                },
            )
            .await
        })
        .map(|rows: Vec<Inventory>| {
            rows.into_iter().next().unwrap_or(Inventory {
                archives: 0,
                oligos: 0,
            })
        })
    }

    /// Un oligo, par son identifiant.
    pub async fn get(&self, id: i64) -> Result<Option<StoredOligo>> {
        on_pool!(self.pool.as_ref(), executor => {
            crate::database::fetch(
                executor,
                "SELECT id, uuid, sequence_data, archive, position FROM sequences WHERE id = $1",
                &[id.into()],
                |row| -> Result<StoredOligo> {
                    let uuid: String = row.try_get("uuid")?;
                    Ok(StoredOligo {
                        id: row.try_get("id")?,
                        uuid: Uuid::parse_str(&uuid).map_err(|e| {
                            StorageError::DatabaseError(format!("identifiant illisible : {e}"))
                        })?,
                        sequence: row.try_get::<String, _>("sequence_data")?.into_bytes(),
                        archive: row.try_get("archive")?,
                        position: usize::try_from(row.try_get::<i64, _>("position")?)
                            .map_err(|e| StorageError::DatabaseError(format!("rang invalide : {e}")))?,
                    })
                },
            )
            .await
        })
        .map(|rows: Vec<StoredOligo>| rows.into_iter().next())
    }

    /// Les archives vérifiant `condition`, avec le nombre de leurs oligos.
    async fn archives_where(
        &self,
        condition: &str,
        arguments: &[Argument],
    ) -> Result<Vec<Archive>> {
        let sql = format!(
            "SELECT a.id, a.file_name, a.sha256, a.scheme, a.stats, a.created_at, \
             (SELECT COUNT(*) FROM sequences s WHERE s.archive = a.id) AS oligos \
             FROM archives a {condition} ORDER BY a.created_at DESC, a.id DESC"
        );
        on_pool!(self.pool.as_ref(), executor => {
            crate::database::fetch(executor, sqlx::AssertSqlSafe(sql), arguments, |row| -> Result<Archive> {
                Ok(Archive {
                    info: ArchiveInfo {
                        id: row.try_get("id")?,
                        file_name: row.try_get("file_name")?,
                        sha256: row.try_get("sha256")?,
                        scheme: row.try_get("scheme")?,
                        stats: row.try_get("stats")?,
                    },
                    oligos: row.try_get("oligos")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .await
        })
    }
}

/// Un lot tient dans la limite de paramètres de SQLite (999 par requête).
const _: () = assert!(BATCH_SIZE * 7 <= 999);
