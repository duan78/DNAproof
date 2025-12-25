//! Affichage des statistiques

use adn_core::{DnaSequence, ConstraintChecker};

/// Affiche les statistiques des séquences
#[allow(dead_code)]
pub fn display_stats(sequences: &[DnaSequence]) {
    if sequences.is_empty() {
        println!("Aucune séquence à afficher");
        return;
    }

    let _checker = ConstraintChecker::new();

    println!("\n┌────────────────────────────────────────────────┐");
    println!("│ Statistiques Globales                          │");
    println!("├────────────────────────────────────────────────┤");

    // Nombre de séquences
    println!("│ Nombre de séquences    : {:>8}              │", sequences.len());

    // Longueur totale
    let total_length: usize = sequences.iter().map(|s| s.len()).sum();
    println!("│ Longueur totale        : {:>8} bases        │", total_length);

    // Longueur moyenne
    let avg_length = total_length / sequences.len();
    println!("│ Longueur moyenne       : {:>8} bases        │", avg_length);

    // GC moyen
    let avg_gc: f64 = sequences.iter().map(|s| s.metadata.gc_ratio).sum::<f64>() / sequences.len() as f64;
    println!("│ GC moyen               : {:>8.1}%              │", avg_gc * 100.0);

    // Entropie moyenne
    let avg_entropy: f64 = sequences.iter().map(|s| s.metadata.entropy).sum::<f64>() / sequences.len() as f64;
    println!("│ Entropie moyenne       : {:>8.2}              │", avg_entropy);

    println!("└────────────────────────────────────────────────┘");
}
