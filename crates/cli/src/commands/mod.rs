//! Commandes CLI

pub mod decode;
pub mod encode;
pub mod simulate;
pub mod visualize;

use adn_core::DnaSequence;
use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Lit un fichier FASTA.
///
/// Les séquences invalides produisent une erreur explicite (une base
/// inconnue indiquait auparavant une séquence silencieusement ignorée, et
/// l'utilisateur voyait moins de séquences que le fichier n'en contenait).
pub(crate) fn read_fasta(path: &Path, source: &str) -> Result<Vec<DnaSequence>> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("Impossible d'ouvrir {}: {}", path.display(), e))?;
    let reader = BufReader::new(file);
    let mut sequences = Vec::new();

    let mut current_seq = String::new();
    let mut chunk_index = 0;

    for line in reader.lines() {
        let line = line?.trim().to_string();

        if line.is_empty() {
            continue;
        }

        if line.starts_with('>') {
            if !current_seq.is_empty() {
                let seq = DnaSequence::from_str(
                    &current_seq,
                    source.to_string(),
                    chunk_index,
                    current_seq.len() / 4,
                    0,
                )
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Séquence #{} invalide dans {}: {}",
                        chunk_index,
                        path.display(),
                        e
                    )
                })?;
                sequences.push(seq);
                chunk_index += 1;
            }
            current_seq = String::new();
        } else {
            current_seq.push_str(&line);
        }
    }

    if !current_seq.is_empty() {
        let seq = DnaSequence::from_str(
            &current_seq,
            source.to_string(),
            chunk_index,
            current_seq.len() / 4,
            0,
        )
        .map_err(|e| {
            anyhow::anyhow!(
                "Séquence #{} invalide dans {}: {}",
                chunk_index,
                path.display(),
                e
            )
        })?;
        sequences.push(seq);
    }

    Ok(sequences)
}
