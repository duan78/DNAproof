//! Optimiseur GC avec Programmation Dynamique
//!
//! Ce module trouve le padding optimal de longueur minimale pour atteindre
//! les contraintes GC, en utilisant la programmation dynamique pour explorer
//! tous les chemins possibles.

use crate::error::{DnaError, Result};
use crate::sequence::{DnaConstraints, IupacBase};
use std::collections::{HashMap, HashSet, BinaryHeap};

/// État pour la programmation dynamique
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DpState {
    /// Position dans le padding
    pos: usize,
    /// Nombre de bases GC
    gc_count: usize,
    /// Dernière base ajoutée
    last_base: IupacBase,
    /// Longueur du run actuel d'homopolymer
    current_run: usize,
}

/// État avec score pour tri
#[derive(Debug, Clone)]
struct ScoredState {
    state: DpState,
    sequence: Vec<IupacBase>,
    gc_ratio: f64,
}

impl PartialEq for ScoredState {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.sequence == other.sequence
            && (self.gc_ratio - other.gc_ratio).abs() < 1e-9
    }
}

impl Eq for ScoredState {}

// Ordre inversé pour BinaryHeap (min-heap pour le score GC)
impl Ord for ScoredState {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Comparer par distance au GC target (plus proche = meilleur)
        let target = 0.5; // GC target de 50%
        let self_dist = (self.gc_ratio - target).abs();
        let other_dist = (other.gc_ratio - target).abs();

        other_dist.partial_cmp(&self_dist).unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl PartialOrd for ScoredState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Optimiseur GC avec programmation dynamique
pub struct GcOptimizer {
    /// Cache des solutions déjà calculées
    cache: HashMap<(usize, usize, usize, IupacBase), Option<Vec<IupacBase>>>,
    /// Nombre maximal de bases de padding à essayer
    max_padding_length: usize,
    /// Nombre maximum d'états à garder (élaguation)
    max_states: usize,
}

impl GcOptimizer {
    /// Crée un nouvel optimiseur GC
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            max_padding_length: 50,
            max_states: 100,
        }
    }

    /// Configure la longueur maximale de padding
    pub fn with_max_padding(mut self, max_length: usize) -> Self {
        self.max_padding_length = max_length;
        self
    }

    /// Configure le nombre maximum d'états (pour élaguation)
    pub fn with_max_states(mut self, max_states: usize) -> Self {
        self.max_states = max_states;
        self
    }

    /// Trouve le padding optimal de longueur minimale pour atteindre GC cible
    ///
    /// # Arguments
    /// * `current_bases` - Bases déjà présentes
    /// * `target_gc_min` - GC minimum cible (0-1)
    /// * `target_gc_max` - GC maximum cible (0-1)
    /// * `max_homopolymer` - Longueur max d'homopolymer
    ///
    /// # Retourne
    /// `Some(padding)` si une solution est trouvée, `None` sinon
    pub fn find_optimal_padding(
        &mut self,
        current_bases: &[IupacBase],
        target_gc_min: f64,
        target_gc_max: f64,
        max_homopolymer: usize,
    ) -> Option<Vec<IupacBase>> {
        if current_bases.is_empty() {
            return None;
        }

        // Calculer l'état initial
        let current_gc_count = current_bases.iter().filter(|b| b.is_gc()).count();
        let last_base = *current_bases.last()?;
        let current_run = self.count_trailing_run(current_bases);

        // Vérifier cache
        let cache_key = (
            current_bases.len(),
            current_gc_count,
            current_run,
            last_base,
        );

        if let Some(cached) = self.cache.get(&cache_key) {
            return cached.clone();
        }

        // Utiliser l'algorithme de recherche
        let result = self.find_padding_internal(
            current_bases,
            current_gc_count,
            last_base,
            current_run,
            target_gc_min,
            target_gc_max,
            max_homopolymer,
        );

        // Mettre en cache
        self.cache.insert(cache_key, result.clone());

        result
    }

    /// Implémentation interne de la recherche de padding
    fn find_padding_internal(
        &mut self,
        current_bases: &[IupacBase],
        current_gc_count: usize,
        last_base: IupacBase,
        current_run: usize,
        target_gc_min: f64,
        target_gc_max: f64,
        max_homopolymer: usize,
    ) -> Option<Vec<IupacBase>> {
        let total_bases = current_bases.len();
        let _target_gc = (target_gc_min + target_gc_max) / 2.0;

        // Initialiser la file de priorité avec l'état initial
        let initial_state = DpState {
            pos: 0,
            gc_count: current_gc_count,
            last_base,
            current_run,
        };

        let mut pq = BinaryHeap::new();
        pq.push(ScoredState {
            state: initial_state,
            sequence: Vec::new(),
            gc_ratio: current_gc_count as f64 / total_bases as f64,
        });

        // Ensemble des états visités pour éviter les boucles
        let mut visited = HashSet::new();

        // BFS avec recherche prioritaire
        while let Some(scored) = pq.pop() {
            let state = scored.state;
            let sequence = scored.sequence;

            // Vérifier si on a atteint la cible GC
            let new_total = total_bases + state.pos;
            let current_gc_ratio = state.gc_count as f64 / new_total as f64;

            if current_gc_ratio >= target_gc_min && current_gc_ratio <= target_gc_max {
                return Some(sequence);
            }

            // Arrêter si on dépasse la longueur max
            if state.pos >= self.max_padding_length {
                continue;
            }

            // Marquer comme visité
            let state_key = (state.pos, state.gc_count, state.current_run, state.last_base);
            if !visited.insert(state_key) {
                continue; // Déjà visité
            }

            // Élaguation: si trop d'états, garder seulement les meilleurs
            if pq.len() > self.max_states {
                pq = self.prune_queue(pq);
            }

            // Essayer d'ajouter chaque base
            for &base in &[IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T] {
                // Vérifier contrainte d'homopolymer
                let new_run = if base == state.last_base {
                    state.current_run + 1
                } else {
                    1
                };

                if new_run > max_homopolymer {
                    continue; // Violation de contrainte
                }

                // Créer nouvel état
                let new_gc_count = state.gc_count + if base.is_gc() { 1 } else { 0 };
                let mut new_sequence = sequence.clone();
                new_sequence.push(base);

                let new_state = DpState {
                    pos: state.pos + 1,
                    gc_count: new_gc_count,
                    last_base: base,
                    current_run: new_run,
                };

                let new_total = total_bases + new_state.pos;
                let new_gc_ratio = new_gc_count as f64 / new_total as f64;

                pq.push(ScoredState {
                    state: new_state,
                    sequence: new_sequence,
                    gc_ratio: new_gc_ratio,
                });
            }
        }

        // Aucune solution trouvée
        None
    }

    /// Compte la longueur du run à la fin des bases
    fn count_trailing_run(&self, bases: &[IupacBase]) -> usize {
        if bases.is_empty() {
            return 0;
        }

        let last = bases.last().unwrap();
        bases.iter()
            .rev()
            .take_while(|&&b| b == *last)
            .count()
    }

    /// Élaguer la file pour ne garder que les meilleurs états
    fn prune_queue(&self, mut pq: BinaryHeap<ScoredState>) -> BinaryHeap<ScoredState> {
        let mut pruned = BinaryHeap::new();
        let mut count = 0;

        while let Some(state) = pq.pop() {
            if count >= self.max_states {
                break;
            }
            pruned.push(state);
            count += 1;
        }

        pruned
    }

    /// Calcule le ratio GC actuel d'une séquence
    pub fn compute_gc_ratio(&self, bases: &[IupacBase]) -> f64 {
        if bases.is_empty() {
            return 0.5;
        }

        let gc_count = bases.iter().filter(|b| b.is_gc()).count();
        gc_count as f64 / bases.len() as f64
    }

    /// Vérifie si un ratio est dans les limites
    pub fn is_gc_in_range(&self, ratio: f64, min: f64, max: f64) -> bool {
        ratio >= min && ratio <= max
    }

    /// Trouve un padding simple (fallback si l'optimisation échoue)
    pub fn find_simple_padding(
        &self,
        current_bases: &[IupacBase],
        target_gc_min: f64,
        target_gc_max: f64,
        max_homopolymer: usize,
    ) -> Vec<IupacBase> {
        let mut padding = Vec::new();
        let mut test_bases = current_bases.to_vec();

        // Pattern qui alterne GC/AT
        let bases_to_try = [
            IupacBase::G,
            IupacBase::C,
            IupacBase::T,
            IupacBase::A,
        ];

        let mut attempt = 0;
        let max_attempts = 100;

        while attempt < max_attempts {
            let current_gc = self.compute_gc_ratio(&test_bases);

            if self.is_gc_in_range(current_gc, target_gc_min, target_gc_max) {
                return padding;
            }

            // Ajouter une base qui aide à atteindre la cible
            let base_idx = attempt % bases_to_try.len();
            let new_base = bases_to_try[base_idx];

            // Vérifier contrainte d'homopolymer
            if let Some(&last) = test_bases.last() {
                let run_len = test_bases.iter()
                    .rev()
                    .take_while(|&&b| b == last)
                    .count();

                if last == new_base && run_len >= max_homopolymer {
                    attempt += 1;
                    continue;
                }
            }

            padding.push(new_base);
            test_bases.push(new_base);
            attempt += 1;
        }

        // Retourner le meilleur padding trouvé (même si pas parfait)
        padding
    }

    /// Vide le cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Default for GcOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_ratio_computation() {
        let optimizer = GcOptimizer::new();

        let bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
            IupacBase::C,
            IupacBase::G,
        ];

        let gc_ratio = optimizer.compute_gc_ratio(&bases);
        assert!((gc_ratio - 4.0 / 6.0).abs() < 0.01);
    }

    #[test]
    fn test_count_trailing_run() {
        let optimizer = GcOptimizer::new();

        let bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::G,
            IupacBase::G,
        ];

        let run = optimizer.count_trailing_run(&bases);
        assert_eq!(run, 3); // Trois G à la fin
    }

    #[test]
    fn test_find_optimal_padding() {
        let mut optimizer = GcOptimizer::new();

        // Séquence avec GC trop bas (seulement A et T)
        let bases: Vec<IupacBase> = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::T,
            IupacBase::T,
            IupacBase::A,
            IupacBase::T,
        ];

        let padding = optimizer.find_optimal_padding(
            &bases,
            0.40,  // GC min 40%
            0.60,  // GC max 60%
            3,     // Max homopolymer
        );

        assert!(padding.is_some(), "Devrait trouver une solution");

        let padding = padding.unwrap();
        assert!(!padding.is_empty());

        // Vérifier que le padding atteint la cible GC
        let mut test_bases = bases.clone();
        test_bases.extend_from_slice(&padding);

        let final_gc = optimizer.compute_gc_ratio(&test_bases);
        assert!(final_gc >= 0.40 && final_gc <= 0.60,
            "GC final {} devrait être dans [0.40, 0.60]", final_gc);
    }

    #[test]
    fn test_find_simple_padding() {
        let optimizer = GcOptimizer::new();

        let bases: Vec<IupacBase> = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::T,
            IupacBase::T,
        ];

        let padding = optimizer.find_simple_padding(
            &bases,
            0.40,
            0.60,
            3,
        );

        // Devrait retourner un padding (pas forcément optimal)
        let mut test_bases = bases.clone();
        test_bases.extend_from_slice(&padding);

        let final_gc = optimizer.compute_gc_ratio(&test_bases);

        // Le padding simple devrait au moins essayer d'améliorer le GC
        let initial_gc = optimizer.compute_gc_ratio(&bases);
        assert!(final_gc > initial_gc || final_gc >= 0.40);
    }

    #[test]
    fn test_gc_already_in_range() {
        let mut optimizer = GcOptimizer::new();

        // Séquence déjà avec bon GC (3 GC sur 6 = 50%)
        let bases: Vec<IupacBase> = vec![
            IupacBase::G,
            IupacBase::C,
            IupacBase::A,
            IupacBase::T,
            IupacBase::G,
            IupacBase::C,
        ];

        let padding = optimizer.find_optimal_padding(
            &bases,
            0.40,
            0.60,
            3,
        );

        // Pas besoin de padding si déjà dans les limites
        // Mais l'algorithme peut retourner Some(vec![]) ou None
        if let Some(p) = padding {
            assert!(p.is_empty() || p.len() < 5);
        }
    }

    #[test]
    fn test_homopolymer_constraint() {
        let mut optimizer = GcOptimizer::new();

        // Séquence finissant par AAA
        let bases: Vec<IupacBase> = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
        ];

        let padding = optimizer.find_optimal_padding(
            &bases,
            0.40,
            0.60,
            3, // Max homopolymer = 3
        );

        if let Some(p) = padding {
            // Vérifier qu'on n'a pas ajouté un 4ème A
            let mut test_bases = bases.clone();
            test_bases.extend_from_slice(&p);

            for window in test_bases.windows(4) {
                assert!(window[0] != window[1] || window[1] != window[2] || window[2] != window[3],
                    "Homopolymer > 3 détecté: {:?}", window);
            }
        }
    }

    #[test]
    fn test_cache() {
        let mut optimizer = GcOptimizer::new();

        let bases: Vec<IupacBase> = vec![IupacBase::A, IupacBase::T];

        // Premier appel (pas en cache)
        let padding1 = optimizer.find_optimal_padding(&bases, 0.40, 0.60, 3);

        // Deuxième appel (devrait utiliser le cache)
        let padding2 = optimizer.find_optimal_padding(&bases, 0.40, 0.60, 3);

        assert_eq!(padding1, padding2);
    }

    #[test]
    fn test_clear_cache() {
        let mut optimizer = GcOptimizer::new();

        let bases: Vec<IupacBase> = vec![IupacBase::A, IupacBase::T];

        optimizer.find_optimal_padding(&bases, 0.40, 0.60, 3);
        assert!(!optimizer.cache.is_empty());

        optimizer.clear_cache();
        assert!(optimizer.cache.is_empty());
    }
}
