//! Échantillonnage LT-code partagé entre l'encodeur et le décodeur Fountain.
//!
//! Ces fonctions DOIVENT rester strictement symétriques entre encode et
//! decode : le décodeur reconstruit le degré et les indices de chaque droplet
//! en rejouant les mêmes tirages pseudo-aléatoires à partir du seed stocké
//! dans la séquence. C'est pourquoi elles vivent dans ce module unique plutôt
//! que dupliquées dans encoder.rs / decoder.rs.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::collections::HashSet;

/// Constante mélangée au seed pour le flux RNG de sélection des indices.
///
/// Sans elle, le degré et les indices dériveraient du même premier mot 64
/// bits du générateur (même seed ⇒ même premier bloc ChaCha) : degré et
/// premier index seraient statistiquement corrélés, et certains sous-ensembles
/// de chunks n'apparaîtraient jamais dans des droplets de faible degré —
/// le peeling decoder échouerait alors faute de points d'entrée (observé
/// systématiquement pour K=5 chunks : chunks 0/3/4 absents des degrés 1).
const INDEX_STREAM_XOR: u64 = 0x9E37_79B9_7F4A_7C15;

/// Échantillonne un degré depuis la distribution Robust Soliton simplifiée.
///
/// K = num_chunks, c = 0.1 ; le paramètre δ de la distribution théorique
/// n'est pas utilisé dans cette implémentation simplifiée (τ sans segment
/// logarithmique médian).
pub fn sample_robust_soliton_degree(num_chunks: usize, seed: u64) -> usize {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let k = num_chunks as f64;
    let c = 0.1;

    // Tau function
    let tau = |d: f64| -> f64 {
        if d <= (k / c - 1.0).ceil() {
            1.0 / (d * c)
        } else {
            0.0
        }
    };

    // Calculer les poids pour chaque degré possible
    let mut weights = Vec::with_capacity(num_chunks);

    for d in 1..=num_chunks {
        let d_float = d as f64;
        let rho = if d == 1 {
            1.0 / k
        } else {
            1.0 / (d_float * (d_float - 1.0))
        };

        let weight = rho + tau(d as f64);
        weights.push(weight);
    }

    // Normaliser
    let sum: f64 = weights.iter().sum();
    for w in weights.iter_mut() {
        *w /= sum;
    }

    // Échantillonner
    let mut cumulative = 0.0;
    let sample = rng.gen::<f64>();

    for (d, &w) in weights.iter().enumerate() {
        cumulative += w;
        if sample <= cumulative {
            return d + 1; // +1 car les degrés commencent à 1
        }
    }

    num_chunks // Fallback
}

/// Sélectionne `degree` indices de chunks distincts, déterministes par seed.
///
/// Le flux RNG est dérivé du seed via `INDEX_STREAM_XOR` pour être
/// indépendant du tirage du degré (voir la constante).
pub fn select_chunk_indices(num_chunks: usize, degree: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ INDEX_STREAM_XOR);
    let mut indices = HashSet::new();

    while indices.len() < degree {
        let idx = rng.gen_range(0..num_chunks);
        indices.insert(idx);
    }

    let mut result: Vec<usize> = indices.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_degree_and_indices_are_deterministic() {
        for seed in [0u64, 1, 42, 999] {
            let d1 = sample_robust_soliton_degree(10, seed);
            let d2 = sample_robust_soliton_degree(10, seed);
            assert_eq!(d1, d2);

            let i1 = select_chunk_indices(10, d1, seed);
            let i2 = select_chunk_indices(10, d1, seed);
            assert_eq!(i1, i2);
        }
    }

    #[test]
    fn test_all_chunks_reachable_in_degree_one() {
        // Régression (corrélation degré/indices) : sur un grand nombre de
        // seeds, chaque chunk doit apparaître dans des droplets de degré 1.
        // Avec l'ancien tirage, seuls 2 chunks sur 5 étaient atteignables.
        let num_chunks = 5;
        let mut seen_in_degree_one = vec![false; num_chunks];

        for seed in 0..2000u64 {
            let degree = sample_robust_soliton_degree(num_chunks, seed);
            if degree == 1 {
                for idx in select_chunk_indices(num_chunks, degree, seed) {
                    seen_in_degree_one[idx] = true;
                }
            }
        }

        assert!(
            seen_in_degree_one.iter().all(|&s| s),
            "chunks jamais couverts en degré 1: {:?}",
            seen_in_degree_one
                .iter()
                .enumerate()
                .filter(|(_, &s)| !s)
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_peeling_coverage_k5() {
        // Avec 5x redondance, le peeling doit pouvoir démarrer sur chaque
        // chunk : vérifie que le graphe généré n'a pas de chunk isolé des
        // petits degrés (chaque chunk dans au moins un droplet de degré <= 2).
        let num_chunks = 5;
        let mut covered = vec![false; num_chunks];

        for seed in 0..100u64 {
            let degree = sample_robust_soliton_degree(num_chunks, seed);
            if degree <= 2 {
                for idx in select_chunk_indices(num_chunks, degree, seed) {
                    covered[idx] = true;
                }
            }
        }

        assert!(
            covered.iter().all(|&c| c),
            "chunks non couverts par les petits degrés: {:?}",
            covered
        );
    }
}
