-- Colonnes propres aux archives v2, et suppression de deux tables jamais utilisées.

ALTER TABLE sequences ADD COLUMN archive TEXT NOT NULL DEFAULT '';
ALTER TABLE sequences ADD COLUMN position BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_sequences_archive ON sequences(archive, position);

DROP TABLE IF EXISTS search_index;
DROP TABLE IF EXISTS storage_metadata;
