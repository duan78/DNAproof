//! Visualisation des séquences

use adn_core::DnaSequence;

/// Affiche les séquences
#[allow(dead_code)]
pub fn display_sequences(sequences: &[DnaSequence]) {
    for (i, seq) in sequences.iter().enumerate() {
        println!("Séquence {}/{}", i + 1, sequences.len());
        println!("  ID: {}", seq.id);
        println!("  Longueur: {} bases", seq.len());
        println!("  GC: {:.1}%", seq.metadata.gc_ratio * 100.0);
        println!("  Sequence: {}", seq.to_string().chars().take(50).collect::<String>());
        println!();
    }
}
