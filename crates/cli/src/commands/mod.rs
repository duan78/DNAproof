//! Commandes de la ligne de commande.

pub mod decode;
pub mod encode;
pub mod simulate;
pub mod verify;
pub mod visualize;

use adn_core::fasta::{self, Record};
use anyhow::{bail, Context};
use std::path::{Path, PathBuf};

/// Lit des fichiers de lectures et refuse nettement les archives de la version 0.1.
pub(crate) fn read_reads(paths: &[PathBuf]) -> anyhow::Result<Vec<Record>> {
    let mut records = Vec::new();
    for path in paths {
        let loaded = fasta::read_file(path)
            .with_context(|| format!("impossible de lire {}", path.display()))?;
        if loaded.is_empty() {
            bail!("{} ne contient aucune séquence", path.display());
        }
        records.extend(loaded);
    }
    if fasta::is_legacy_v1(&records) {
        bail!("{}", fasta::LEGACY_HINT);
    }
    Ok(records)
}

/// Séquences seules : les en-têtes ne portent aucune information utile au décodage.
pub(crate) fn sequences(records: &[Record]) -> Vec<Vec<u8>> {
    records
        .iter()
        .map(|record| record.sequence.clone())
        .collect()
}

/// Nom du fichier, pour l'affichage.
pub(crate) fn name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
        .to_string()
}
