//! Fonctions mathématiques

use adn_core::IupacBase;

/// Configuration pour le calcul d'entropie
#[derive(Debug, Clone, Copy)]
pub struct EntropyConfig {
    /// Base du logarithm (2.0 pour bits, e pour nats)
    pub log_base: f64,
}

impl Default for EntropyConfig {
    fn default() -> Self {
        Self { log_base: 2.0 }
    }
}

/// Calcule l'entropie de Shannon d'une séquence
pub fn entropy(bases: &[IupacBase], _config: Option<EntropyConfig>) -> f64 {
    if bases.is_empty() {
        return 0.0;
    }

    // Compter les fréquences
    let mut freq = [0usize; 4]; // A, C, G, T

    for base in bases {
        match base {
            IupacBase::A => freq[0] += 1,
            IupacBase::C => freq[1] += 1,
            IupacBase::G => freq[2] += 1,
            IupacBase::T => freq[3] += 1,
            _ => {}
        }
    }

    // Calculer l'entropie
    let len = bases.len() as f64;
    let mut entropy = 0.0;

    for &count in &freq {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Calcule la complexité d'une séquence (nombre de patterns uniques de longueur k)
pub fn complexity(bases: &[IupacBase], k: usize) -> f64 {
    if bases.len() < k || k == 0 {
        return 0.0;
    }

    let mut patterns = std::collections::HashSet::new();

    for window in bases.windows(k) {
        let pattern: String = window.iter().map(|b| b.as_char()).collect();
        patterns.insert(pattern);
    }

    // Complexité = nombre de patterns uniques / nombre total possible
    let total_possible = 4f64.powi(k as i32);
    patterns.len() as f64 / total_possible
}

/// Calcule le contenu GC d'une séquence
pub fn gc_content(bases: &[IupacBase]) -> f64 {
    if bases.is_empty() {
        return 0.5;
    }

    let gc_count = bases.iter().filter(|b| b.is_gc()).count();
    gc_count as f64 / bases.len() as f64
}

/// Calcule la distance de Hamming entre deux séquences
pub fn hamming_distance(seq1: &[IupacBase], seq2: &[IupacBase]) -> Result<usize, String> {
    if seq1.len() != seq2.len() {
        return Err("Séquences de longueurs différentes".to_string());
    }

    let distance = seq1
        .iter()
        .zip(seq2.iter())
        .filter(|(b1, b2)| b1 != b2)
        .count();

    Ok(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entropy() {
        let bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
        ];

        // Entropie minimale (toutes les mêmes bases)
        let e = entropy(&bases, None);
        assert!((e - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_entropy_max() {
        let bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
        ];

        // Entropie maximale (distribution uniforme)
        let e = entropy(&bases, None);
        assert!((e - 2.0).abs() < 1e-9); // log2(4) = 2
    }

    #[test]
    fn test_gc_content() {
        let bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
        ];

        let gc = gc_content(&bases);
        assert!((gc - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_hamming_distance() {
        let seq1 = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
        ];

        let seq2 = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::A,
            IupacBase::T,
        ];

        let dist = hamming_distance(&seq1, &seq2).unwrap();
        assert_eq!(dist, 1); // Seule la 3ème base diffère
    }

    #[test]
    fn test_complexity() {
        let bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
        ];

        // Faible complexité (répétition)
        let c = complexity(&bases, 2);
        assert!(c < 0.5);
    }
}
