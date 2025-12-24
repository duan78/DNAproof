-- Initialisation de la base de données PostgreSQL

CREATE TABLE IF NOT EXISTS sequences (
    id SERIAL PRIMARY KEY,
    uuid UUID NOT NULL UNIQUE,
    sequence_data TEXT NOT NULL,
    metadata JSONB NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sequences_uuid ON sequences(uuid);
CREATE INDEX IF NOT EXISTS idx_sequences_created_at ON sequences(created_at);

-- Table pour les métadonnées de stockage
CREATE TABLE IF NOT EXISTS storage_metadata (
    id SERIAL PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL
);

-- Table pour les index de recherche
CREATE TABLE IF NOT EXISTS search_index (
    id SERIAL PRIMARY KEY,
    sequence_id INTEGER NOT NULL,
    search_term TEXT NOT NULL,
    FOREIGN KEY (sequence_id) REFERENCES sequences(id) ON DELETE CASCADE,
    UNIQUE(sequence_id, search_term)
);

CREATE INDEX IF NOT EXISTS idx_search_index_term ON search_index(search_term);
CREATE INDEX IF NOT EXISTS idx_search_index_sequence ON search_index(sequence_id);

-- Extension pour la recherche full-text
CREATE EXTENSION IF NOT EXISTS pg_trgm;