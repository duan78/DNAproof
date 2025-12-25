//! Commande de décodage

use crate::create_spinner;
use adn_core::{Decoder, DecoderConfig, DnaSequence};
use anyhow::Result;
use std::path::PathBuf;
use std::fs::File;
use std::io::{BufRead, BufReader};

pub fn run(input: PathBuf, output: PathBuf, ignore_checksum: bool) -> Result<()> {
    println!("🧬 Décodage de: {}", input.display());

    // 1. Lire le fichier FASTA
    let spinner = create_spinner("Lecture des séquences...");
    let sequences = read_fasta(&input)?;
    spinner.finish_with_message(format!("{} séquences lues", sequences.len()));

    // 2. Configurer le décodeur
    let config = DecoderConfig {
        ignore_checksum,
        max_iterations: 10000,
        auto_decompress: true,
        compression_type: adn_core::codec::decoder::CompressionType::Auto,
    };

    // 3. Décoder
    let spinner = create_spinner("Décodage...");
    let decoder = Decoder::new(config);
    let data = decoder.decode(&sequences)?;
    spinner.finish_with_message(format!("Données récupérées ({} octets)", data.len()));

    // 4. Écrire le fichier de sortie
    let spinner = create_spinner("Écriture du fichier...");
    std::fs::write(&output, &data)?;
    spinner.finish_with_message(format!("Fichier écrit: {}", output.display()));

    println!("\n✅ Décodage terminé!");

    Ok(())
}

/// Lit un fichier FASTA
fn read_fasta(path: &PathBuf) -> Result<Vec<DnaSequence>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut sequences = Vec::new();

    let mut current_id: Option<String> = None;
    let mut current_seq = String::new();
    let mut chunk_index = 0;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if let Some(stripped) = line.strip_prefix('>') {
            // Sauvegarder la séquence précédente
            if !current_seq.is_empty() {
                if let Ok(seq) = DnaSequence::from_str(
                    &current_seq,
                    current_id.clone().unwrap_or("unknown".to_string()),
                    chunk_index,
                    current_seq.len() / 4, // Estimation
                    0,
                ) {
                    sequences.push(seq);
                    chunk_index += 1;
                }
            }

            // Extraire l'ID de la ligne header
            let parts: Vec<&str> = stripped.split('|').collect();
            current_id = Some(parts[0].to_string());
            current_seq = String::new();
        } else {
            current_seq.push_str(line);
        }
    }

    // Dernière séquence
    if !current_seq.is_empty() {
        if let Ok(seq) = DnaSequence::from_str(
            &current_seq,
            current_id.unwrap_or("unknown".to_string()),
            chunk_index,
            current_seq.len() / 4,
            0,
        ) {
            sequences.push(seq);
        }
    }

    Ok(sequences)
}
