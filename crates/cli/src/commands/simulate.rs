//! Commande de simulation

use crate::create_progress_bar;
use adn_simulation::{DnaChannel, ChannelConfig, ErrorModel, MetricsCollector};
use adn_core::DnaSequence;
use anyhow::Result;
use std::path::PathBuf;
use std::io::{BufRead, BufReader};

pub fn run(
    input: PathBuf,
    substitution_rate: f64,
    insertion_rate: f64,
    deletion_rate: f64,
    iterations: usize,
) -> Result<()> {
    println!("🧬 Simulation d'erreurs sur: {}", input.display());

    // 1. Lire les séquences
    let sequences = read_fasta(&input)?;
    println!("{} séquences chargées", sequences.len());

    // 2. Configurer le canal
    let error_model = ErrorModel {
        substitution_rate,
        insertion_rate,
        deletion_rate,
        seed: 42,
    };

    let config = ChannelConfig {
        error_model,
        temperature: 25.0,
        ph: 7.0,
        storage_duration_days: 30,
    };

    // 3. Simuler
    let pb = create_progress_bar((iterations * sequences.len()) as u64, "Simulation en cours...");
    let mut channel = DnaChannel::new(config);
    let mut collector = MetricsCollector::new();

    for seq in &sequences {
        for _ in 0..iterations {
            let (corrupted, metrics) = channel.transmit(seq)?;
            collector.add(metrics);
        }
        pb.inc(1);
    }

    pb.finish_with_message(String::from("Simulation terminée"));

    // 4. Afficher les résultats
    println!("\n📊 Résultats de la simulation:");
    println!("{}", collector.average().format_table());

    println!("\n📈 Statistiques agrégées:");
    println!("   Minimum:");
    println!("{}", collector.min().format_table());

    println!("\n   Maximum:");
    println!("{}", collector.max().format_table());

    println!("\n✅ Simulation terminée!");

    Ok(())
}

/// Lit un fichier FASTA (version simplifiée)
fn read_fasta(path: &PathBuf) -> Result<Vec<DnaSequence>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut sequences = Vec::new();

    let mut current_seq = String::new();
    let mut chunk_index = 0;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with('>') {
            if !current_seq.is_empty() {
                if let Ok(seq) = DnaSequence::from_str(
                    &current_seq,
                    "simulated".to_string(),
                    chunk_index,
                    current_seq.len() / 4,
                    0,
                ) {
                    sequences.push(seq);
                    chunk_index += 1;
                }
            }
            current_seq = String::new();
        } else {
            current_seq.push_str(line);
        }
    }

    if !current_seq.is_empty() {
        if let Ok(seq) = DnaSequence::from_str(
            &current_seq,
            "simulated".to_string(),
            chunk_index,
            current_seq.len() / 4,
            0,
        ) {
            sequences.push(seq);
        }
    }

    Ok(sequences)
}
