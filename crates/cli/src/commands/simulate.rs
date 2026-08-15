//! Commande de simulation

use crate::commands::read_fasta;
use crate::create_progress_bar;
use adn_simulation::{ChannelConfig, DnaChannel, ErrorModel, MetricsCollector};
use anyhow::Result;
use std::path::PathBuf;

pub fn run(
    input: PathBuf,
    substitution_rate: f64,
    insertion_rate: f64,
    deletion_rate: f64,
    iterations: usize,
) -> Result<()> {
    println!("🧬 Simulation d'erreurs sur: {}", input.display());

    // 1. Lire les séquences
    let sequences = read_fasta(&input, "simulated")?;
    println!("{} séquences chargées", sequences.len());

    if sequences.is_empty() {
        anyhow::bail!("Aucune séquence trouvée dans {}", input.display());
    }

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

    // 3. Simuler — la barre progresse d'une unité par séquence traitée
    // (toutes ses itérations de transmission incluses).
    let pb = create_progress_bar(sequences.len() as u64, "Simulation en cours...");
    let mut channel = DnaChannel::new(config);
    let mut collector = MetricsCollector::new();

    for seq in &sequences {
        for _ in 0..iterations {
            let (_corrupted, metrics) = channel.transmit(seq)?;
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
