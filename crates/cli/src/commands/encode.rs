//! Commande d'encodage

use crate::{create_progress_bar, create_spinner, CompressionAlgorithm, EncodingAlgorithm};
use adn_core::codec::encoder::{CompressionType, EncoderType};
use adn_core::{DnaConstraints, Encoder, EncoderConfig};
use anyhow::Result;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

pub fn run(
    input: PathBuf,
    output: PathBuf,
    algorithm: EncodingAlgorithm,
    redundancy: f64,
    compress: bool,
    compression: Option<CompressionAlgorithm>,
) -> Result<()> {
    println!("🧬 Encodage de: {}", input.display());

    // 1. Lire le fichier
    let spinner = create_spinner("Lecture du fichier...");
    let data = std::fs::read(&input)?;
    spinner.finish_with_message(format!("Fichier lu ({} octets)", data.len()));

    if data.is_empty() {
        anyhow::bail!("Le fichier d'entrée est vide");
    }

    // 2. Configurer l'encodeur
    let encoder_type = match algorithm {
        EncodingAlgorithm::Ez2017 => EncoderType::ErlichZielinski2017,
        EncodingAlgorithm::Fountain => EncoderType::Fountain,
        EncodingAlgorithm::Goldman => EncoderType::Goldman,
        EncodingAlgorithm::Goldman2013 => EncoderType::Goldman2013,
        EncodingAlgorithm::Grass2015 => EncoderType::Grass2015,
        EncodingAlgorithm::Adaptive => EncoderType::Adaptive,
        EncodingAlgorithm::Base3 => EncoderType::Base3,
        EncodingAlgorithm::Ultimate => EncoderType::Ultimate,
    };

    let compression_type = match compression.unwrap_or(CompressionAlgorithm::Lz4) {
        CompressionAlgorithm::Lz4 => CompressionType::Lz4,
        CompressionAlgorithm::Zstd => CompressionType::Zstd,
        CompressionAlgorithm::None => CompressionType::None,
    };

    // Use lenient constraints for algorithms that don't enforce GC/homopolymer limits
    // Each algorithm has different requirements:
    // - Grass2015: Uses padding that may not meet strict GC constraints
    // - Goldman2013: Uses rotation but may produce sequences outside 40-60% GC
    // - Fountain: May fail to find valid bases with strict constraints
    // - Adaptive: Falls back to Fountain
    // - Goldman (legacy): Simple encoding without GC optimization
    let constraints = match algorithm {
        EncodingAlgorithm::Ez2017 => DnaConstraints {
            // EZ 2017 : contraintes du papier (GC 40-60%, homopolymer < 4),
            // longueur 152nt + marge pour l'encodage rotatif
            gc_min: 0.40,
            gc_max: 0.60,
            max_homopolymer: 3,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        EncodingAlgorithm::Grass2015 => DnaConstraints {
            gc_min: 0.0,
            gc_max: 1.0,
            max_homopolymer: 150,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        EncodingAlgorithm::Goldman2013 | EncodingAlgorithm::Goldman => DnaConstraints {
            gc_min: 0.20, // More lenient for Goldman's rotation-based encoding
            gc_max: 0.80,
            max_homopolymer: 6,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        EncodingAlgorithm::Fountain | EncodingAlgorithm::Adaptive => DnaConstraints {
            gc_min: 0.0, // Très souple - l'encodage direct préserve les données
            gc_max: 1.0,
            max_homopolymer: 150, // Très souple pour éviter les erreurs de validation
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        EncodingAlgorithm::Ultimate => DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 10,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        _ => DnaConstraints::default(),
    };

    let config = EncoderConfig {
        encoder_type,
        chunk_size: 32,
        redundancy,
        compression_enabled: compress,
        compression_type,
        constraints,
    };

    // 3. Encoder
    let pb = create_progress_bar(data.len() as u64, "Encodage ADN...");
    let encoder = Encoder::new(config)?;
    let sequences = encoder.encode(&data)?;
    pb.finish_with_message(format!("{} séquences générées", sequences.len()));

    // 4. Créer le répertoire de sortie
    std::fs::create_dir_all(&output)?;

    // 5. Écrire les séquences en format FASTA
    let output_file = output.join(format!(
        "{}.fasta",
        input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
    ));

    let spinner = create_spinner("Écriture des séquences...");
    let mut file = File::create(&output_file)?;

    for seq in &sequences {
        writeln!(file, "{}", seq.to_fasta())?;
    }

    spinner.finish_with_message(format!("Séquences écrites dans {}", output_file.display()));

    // 6. Statistiques
    println!("\n📊 Statistiques:");
    println!("   Séquences générées: {}", sequences.len());
    println!(
        "   Longueur moyenne: {:.1} bases",
        sequences.iter().map(|s| s.len()).sum::<usize>() as f64 / sequences.len() as f64
    );
    println!(
        "   GC moyen: {:.1}%",
        sequences.iter().map(|s| s.metadata.gc_ratio).sum::<f64>() * 100.0 / sequences.len() as f64
    );

    println!("\n✅ Encodage terminé!");

    Ok(())
}
