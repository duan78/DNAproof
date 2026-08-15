-- Initialisation de la base de données PostgreSQL
--
-- Note: les types correspondent au décodage du repository Rust (DbSequence),
-- qui lit uuid / metadata / created_at / updated_at comme des TEXT
-- (timestamps ISO 8601 produits par chrono::Utc::now().to_rfc3339()).

CREATE TABLE IF NOT EXISTS sequences (
    id BIGSERIAL PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    sequence_data TEXT NOT NULL,
    metadata TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sequences_uuid ON sequences(uuid);
CREATE INDEX IF NOT EXISTS idx_sequences_created_at ON sequences(created_at);

-- Table pour les métadonnées de stockage
CREATE TABLE IF NOT EXISTS storage_metadata (
    id BIGSERIAL PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Table pour les index de recherche
CREATE TABLE IF NOT EXISTS search_index (
    id BIGSERIAL PRIMARY KEY,
    sequence_id BIGINT NOT NULL,
    search_term TEXT NOT NULL,
    FOREIGN KEY (sequence_id) REFERENCES sequences(id) ON DELETE CASCADE,
    UNIQUE(sequence_id, search_term)
);

CREATE INDEX IF NOT EXISTS idx_search_index_term ON search_index(search_term);
CREATE INDEX IF NOT EXISTS idx_search_index_sequence ON search_index(sequence_id);
