//! Commande de décodage

use crate::create_spinner;
use adn_core::{Decoder, DecoderConfig};
use anyhow::Result;
use std::path::PathBuf;

pub fn run(input: PathBuf, output: PathBuf, ignore_checksum: bool) -> Result<()> {
    println!("🧬 Décodage de: {}", input.display());

    // 1. Configurer le décodeur
    let config = DecoderConfig {
        ignore_checksum,
        max_iterations: 10000,
        auto_decompress: true,
        compression_type: adn_core::codec::decoder::CompressionType::Auto,
    };

    // 2. Décoder automatiquement (détecte le schéma depuis les headers FASTA)
    let spinner = create_spinner("Décodage...");
    let decoder = Decoder::new(config);
    // to_string_lossy : les chemins Windows peuvent contenir des caractères
    // non-UTF8 (uncode étendu) — unwrap() paniquait dans ce cas
    let data = decoder.decode_from_fasta_auto(&input.to_string_lossy())?;
    spinner.finish_with_message(format!("Données récupérées ({} octets)", data.len()));

    // 3. Écrire le fichier de sortie
    let spinner = create_spinner("Écriture du fichier...");
    std::fs::write(&output, &data)?;
    spinner.finish_with_message(format!("Fichier écrit: {}", output.display()));

    println!("\n✅ Décodage terminé!");

    Ok(())
}
