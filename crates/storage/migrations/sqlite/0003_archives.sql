-- Une archive : les oligos produits en une fois, et de quoi les retrouver.
--
-- Les lignes de `sequences` portent le nom de leur archive (migration 0002) ; cette table
-- décrit chaque archive (fichier d'origine, empreinte, schéma, mesures), pour la lister et la
-- supprimer après un redémarrage du serveur.

CREATE TABLE IF NOT EXISTS archives (
    id TEXT PRIMARY KEY,
    file_name TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    scheme TEXT NOT NULL,
    -- Mesures de l'encodage, en JSON.
    stats TEXT NOT NULL DEFAULT '{}',
    created_at TIMESTAMP NOT NULL
);
