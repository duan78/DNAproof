//! Validation et gestion des contraintes ADN

use crate::error::{DnaError, Result};
use crate::sequence::{DnaConstraints, IupacBase};

/// Trouve la longueur maximale d'homopolymer dans une séquence
pub fn find_max_homopolymer(bases: &[IupacBase]) -> usize {
    if bases.is_empty() {
        return 0;
    }

    let mut max_run = 1;
    let mut current_run = 1;

    for window in bases.windows(2) {
        if window[0] == window[1] {
            current_run += 1;
            max_run = max_run.max(current_run);
        } else {
            current_run = 1;
        }
    }

    max_run
}

/// Validateur de contraintes ADN
pub struct DnaConstraintValidator {
    constraints: DnaConstraints,
}

impl DnaConstraintValidator {
    /// Crée un nouveau validateur avec les contraintes par défaut
    pub fn new() -> Self {
        Self {
            constraints: DnaConstraints::default(),
        }
    }

    /// Crée un validateur avec des contraintes personnalisées
    pub fn with_constraints(constraints: DnaConstraints) -> Self {
        Self { constraints }
    }

    /// Retourne les contraintes actuelles
    pub fn constraints(&self) -> &DnaConstraints {
        &self.constraints
    }

    /// Modifie les contraintes
    pub fn set_constraints(&mut self, constraints: DnaConstraints) {
        self.constraints = constraints;
    }

    /// Valide une séquence complète
    pub fn validate_sequence(&self, bases: &[IupacBase]) -> Result<()> {
        self.constraints.validate(bases)
    }

    /// Calcule le ratio GC d'une séquence
    pub fn compute_gc_ratio(&self, bases: &[IupacBase]) -> f64 {
        if bases.is_empty() {
            return 0.5;
        }

        let gc_count = bases.iter().filter(|b| b.is_gc()).count();
        gc_count as f64 / bases.len() as f64
    }

    /// Détecte les runs d'homopolymers
    pub fn detect_homopolymers(&self, bases: &[IupacBase]) -> Vec<(usize, IupacBase, usize)> {
        let mut runs = Vec::new();
        let mut current_run = 0;
        let mut start_idx = 0;
        let mut last_base: Option<IupacBase> = None;

        for (i, base) in bases.iter().enumerate() {
            if Some(*base) == last_base {
                current_run += 1;
            } else {
                if current_run > 1 {
                    runs.push((start_idx, last_base.unwrap(), current_run));
                }
                start_idx = i;
                current_run = 1;
                last_base = Some(*base);
            }
        }

        // Ajouter le dernier run
        if current_run > 1 {
            runs.push((start_idx, last_base.unwrap(), current_run));
        }

        runs
    }

    /// Compte la fréquence de chaque base
    pub fn count_bases(&self, bases: &[IupacBase]) -> [usize; 4] {
        let mut counts = [0usize; 4]; // A, C, G, T

        for base in bases {
            match base {
                IupacBase::A => counts[0] += 1,
                IupacBase::C => counts[1] += 1,
                IupacBase::G => counts[2] += 1,
                IupacBase::T => counts[3] += 1,
                _ => {}
            }
        }

        counts
    }

    /// Vérifie si une base peut être ajoutée sans violer les contraintes
    pub fn can_append(&self, bases: &[IupacBase], new_base: IupacBase) -> bool {
        // Vérifier la longueur
        if bases.len() >= self.constraints.max_sequence_length {
            return false;
        }

        // Vérifier l'homopolymer
        if let Some(last_base) = bases.last() {
            if *last_base == new_base {
                // Compter le run actuel
                let run_length = bases
                    .iter()
                    .rev()
                    .take_while(|&&b| b == new_base)
                    .count();

                if run_length >= self.constraints.max_homopolymer {
                    return false;
                }
            }
        }

        // Pour le GC, on ne peut pas savoir à l'avance si ce sera OK
        // car on ne connaît pas la longueur finale
        true
    }

    /// Équilibre le GC content en suggérant une base
    pub fn suggest_base_for_gc(&self, bases: &[IupacBase]) -> Option<IupacBase> {
        let gc_ratio = self.compute_gc_ratio(bases);

        if gc_ratio < self.constraints.gc_min {
            // Besoin de plus de GC
            Some(if rand::random::<bool>() {
                IupacBase::G
            } else {
                IupacBase::C
            })
        } else if gc_ratio > self.constraints.gc_max {
            // Besoin de plus de AT
            Some(if rand::random::<bool>() {
                IupacBase::A
            } else {
                IupacBase::T
            })
        } else {
            // GC est OK, n'importe quelle base
            None
        }
    }

    /// Transforme une séquence pour respecter les contraintes
    pub fn enforce_constraints(&self, bases: &[IupacBase]) -> Result<Vec<IupacBase>> {
        let mut result = Vec::with_capacity(bases.len());

        for &base in bases {
            // Calculer le GC ratio actuel
            let current_gc = self.compute_gc_ratio(&result);

            // Déterminer si on doit forcer un ajustement GC
            let needs_gc_adjustment = if result.len() > 10 {
                // Ne commencer à ajuster qu'après avoir assez de bases
                let target_gc = (self.constraints.gc_min + self.constraints.gc_max) / 2.0;
                let tolerance = (self.constraints.gc_max - self.constraints.gc_min) / 4.0; // Tolérance quart de la plage

                current_gc < target_gc - tolerance || current_gc > target_gc + tolerance
            } else {
                false
            };

            // Choisir la base appropriée
            let chosen_base = if needs_gc_adjustment {
                // Forcer une base qui équilibre le GC
                self.suggest_base_for_gc(&result).unwrap_or(base)
            } else if !self.can_append(&result, base) {
                // Si on ne peut pas ajouter cette base (homopolymer), essayer une alternative
                self.suggest_base_for_gc(&result).unwrap_or_else(|| {
                    // Choisir une base différente de la dernière
                    match result.last() {
                        Some(IupacBase::A) => IupacBase::C,
                        Some(IupacBase::C) => IupacBase::G,
                        Some(IupacBase::G) => IupacBase::T,
                        Some(IupacBase::T) => IupacBase::A,
                        _ => IupacBase::A,
                    }
                })
            } else {
                base
            };

            // Vérifier qu'on peut ajouter la base choisie
            if self.can_append(&result, chosen_base) {
                result.push(chosen_base);
            } else {
                // Dernière tentative: cycle through bases jusqu'à trouver une valide
                let alternatives = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
                let mut found = false;

                for &alt in &alternatives {
                    if alt != chosen_base && self.can_append(&result, alt) {
                        result.push(alt);
                        found = true;
                        break;
                    }
                }

                if !found {
                    return Err(DnaError::ConstraintViolation(
                        "Impossible de satisfaire les contraintes".to_string(),
                    ));
                }
            }
        }

        // Vérification finale et ajustement GC si nécessaire
        let final_gc = self.compute_gc_ratio(&result);

        // Si le GC final est hors limites, essayer de corriger en remplaçant certaines bases
        if final_gc < self.constraints.gc_min || final_gc > self.constraints.gc_max {
            return self.enforce_gc_with_retry(&result, bases);
        }

        self.validate_sequence(&result)?;
        Ok(result)
    }

    /// Corrige le GC content en remplaçant stratégiquement certaines bases
    fn enforce_gc_with_retry(&self, result: &[IupacBase], original: &[IupacBase]) -> Result<Vec<IupacBase>> {
        let mut corrected = result.to_vec();
        let current_gc = self.compute_gc_ratio(&corrected);

        // Déterminer si on a trop ou pas assez de GC
        let needs_more_gc = current_gc < self.constraints.gc_min;
        let target_ratio = (self.constraints.gc_min + self.constraints.gc_max) / 2.0;

        // Identifier les positions candidates pour remplacement
        // On cherche des bases qu'on peut changer sans affecter les homopolymères
        let mut replacement_candidates = Vec::new();

        for i in 0..corrected.len() {
            let base = corrected[i];

            // Vérifier qu'on peut changer cette base sans créer d'homopolymer
            let can_replace = if i > 0 && corrected[i - 1] == base {
                false
            } else if i < corrected.len() - 1 && corrected[i + 1] == base {
                false
            } else {
                true
            };

            if can_replace {
                replacement_candidates.push(i);
            }
        }

        // Mélanger les candidats pour remplacement aléatoire
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        replacement_candidates.shuffle(&mut rng);

        // Remplacer des bases jusqu'à atteindre le target GC
        let max_replacements = (corrected.len() / 10).max(5); // Max 10% des bases
        let mut replacements = 0;

        for &idx in &replacement_candidates {
            if replacements >= max_replacements {
                break;
            }

            let test_gc = self.compute_gc_ratio(&corrected);

            if (test_gc >= self.constraints.gc_min && test_gc <= self.constraints.gc_max)
                || (test_gc - target_ratio).abs() < 0.01 {
                // GC est bon, on arrête
                break;
            }

            let old_base = corrected[idx];
            let needs_gc_now = self.compute_gc_ratio(&corrected) < target_ratio;

            // Choisir une base de remplacement
            let new_base = if needs_gc_now {
                // Besoin de GC, choisir G ou C
                if idx > 0 && corrected[idx - 1] != IupacBase::G {
                    IupacBase::G
                } else {
                    IupacBase::C
                }
            } else {
                // Besoin de AT, choisir A ou T
                if idx > 0 && corrected[idx - 1] != IupacBase::A {
                    IupacBase::A
                } else {
                    IupacBase::T
                }
            };

            // Vérifier que le remplacement ne crée pas d'homopolymer
            let valid_replacement = if idx > 0 && corrected[idx - 1] == new_base {
                false
            } else if idx < corrected.len() - 1 && corrected[idx + 1] == new_base {
                false
            } else {
                true
            };

            if valid_replacement && new_base != old_base {
                corrected[idx] = new_base;
                replacements += 1;
            }
        }

        // Validation finale
        self.validate_sequence(&corrected)?;

        Ok(corrected)
    }
}

impl Default for DnaConstraintValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Checker de contraintes (version simplifiée)
pub struct ConstraintChecker {
    validator: DnaConstraintValidator,
}

impl ConstraintChecker {
    pub fn new() -> Self {
        Self {
            validator: DnaConstraintValidator::new(),
        }
    }

    /// Vérifie rapidement si une séquence est valide
    pub fn is_valid(&self, bases: &[IupacBase]) -> bool {
        self.validator.validate_sequence(bases).is_ok()
    }

    /// Retourne des statistiques sur la séquence
    pub fn stats(&self, bases: &[IupacBase]) -> SequenceStats {
        let counts = self.validator.count_bases(bases);
        let gc_ratio = self.validator.compute_gc_ratio(bases);
        let homopolymers = self.validator.detect_homopolymers(bases);
        let max_homopolymer = homopolymers
            .iter()
            .map(|(_, _, len)| *len)
            .max()
            .unwrap_or(1);

        SequenceStats {
            length: bases.len(),
            count_a: counts[0],
            count_c: counts[1],
            count_g: counts[2],
            count_t: counts[3],
            gc_ratio,
            max_homopolymer,
            homopolymer_count: homopolymers.len(),
        }
    }
}

impl Default for ConstraintChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistiques de séquence
#[derive(Debug, Clone)]
pub struct SequenceStats {
    pub length: usize,
    pub count_a: usize,
    pub count_c: usize,
    pub count_g: usize,
    pub count_t: usize,
    pub gc_ratio: f64,
    pub max_homopolymer: usize,
    pub homopolymer_count: usize,
}

impl SequenceStats {
    /// Affiche les statistiques sous forme de tableau
    pub fn format_table(&self) -> String {
        format!(
            "┌────────────────────────────────────┐\n\
             │ Statistiques de Séquence           │\n\
             ├────────────────────────────────────┤\n\
             │ Longueur    : {:>6} bases        │\n\
             │ A           : {:>6} ({:>5.1}%)    │\n\
             │ C           : {:>6} ({:>5.1}%)    │\n\
             │ G           : {:>6} ({:>5.1}%)    │\n\
             │ T           : {:>6} ({:>5.1}%)    │\n\
             │ GC Ratio    : {:>6.1}%            │\n\
             │ Max Homopoly: {:>6}               │\n\
             │ Nb Homopoly: {:>6}               │\n\
             └────────────────────────────────────┘",
            self.length,
            self.count_a,
            100.0 * self.count_a as f64 / self.length as f64,
            self.count_c,
            100.0 * self.count_c as f64 / self.length as f64,
            self.count_g,
            100.0 * self.count_g as f64 / self.length as f64,
            self.count_t,
            100.0 * self.count_t as f64 / self.length as f64,
            100.0 * self.gc_ratio,
            self.max_homopolymer,
            self.homopolymer_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_ratio_computation() {
        let validator = DnaConstraintValidator::new();
        let bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
            IupacBase::C,
            IupacBase::G,
        ];

        let gc_ratio = validator.compute_gc_ratio(&bases);
        assert_eq!(gc_ratio, 4.0/6.0); // 4 GC (C, G, C, G) sur 6 bases
    }

    #[test]
    fn test_homopolymer_detection() {
        let validator = DnaConstraintValidator::new();
        let bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::C,
            IupacBase::C,
            IupacBase::G,
        ];

        let runs = validator.detect_homopolymers(&bases);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], (0, IupacBase::A, 3));
        assert_eq!(runs[1], (3, IupacBase::C, 2));
    }

    #[test]
    fn test_can_append() {
        let validator = DnaConstraintValidator::new();
        let bases = vec![IupacBase::A, IupacBase::A, IupacBase::A];

        // Ne devrait pas pouvoir ajouter un 4ème A
        assert!(!validator.can_append(&bases, IupacBase::A));

        // Mais peut ajouter une autre base
        assert!(validator.can_append(&bases, IupacBase::C));
    }

    #[test]
    fn test_enforce_constraints() {
        let validator = DnaConstraintValidator::new();

        // Séquence avec homopolymer trop long
        let bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
        ];

        let result = validator.enforce_constraints(&bases);
        assert!(result.is_ok());

        let enforced = result.unwrap();
        assert!(validator.validate_sequence(&enforced).is_ok());
    }

    #[test]
    fn test_stats() {
        let checker = ConstraintChecker::new();
        let bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::C,
            IupacBase::C,
            IupacBase::G,
            IupacBase::G,
            IupacBase::T,
            IupacBase::T,
        ];

        let stats = checker.stats(&bases);
        assert_eq!(stats.length, 8);
        assert_eq!(stats.count_a, 2);
        assert_eq!(stats.count_c, 2);
        assert_eq!(stats.count_g, 2);
        assert_eq!(stats.count_t, 2);
        assert_eq!(stats.gc_ratio, 0.5);
        assert_eq!(stats.max_homopolymer, 2);
    }
}
