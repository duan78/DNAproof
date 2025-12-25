//! Modèles d'erreur pour la simulation

use serde::{Deserialize, Serialize};

/// Type d'erreur ADN
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorType {
    /// Substitution (A→G)
    Substitution,
    /// Insertion
    Insertion,
    /// Délétion
    Deletion,
}

/// Modèle d'erreur pour la simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorModel {
    /// Taux de substitution (par base)
    pub substitution_rate: f64,

    /// Taux d'insertion (par base)
    pub insertion_rate: f64,

    /// Taux de délétion (par base)
    pub deletion_rate: f64,

    /// Seed pour reproductibilité
    pub seed: u64,
}

impl Default for ErrorModel {
    fn default() -> Self {
        Self {
            substitution_rate: 0.01,  // 1%
            insertion_rate: 0.005,    // 0.5%
            deletion_rate: 0.005,     // 0.5%
            seed: 42,
        }
    }
}

impl ErrorModel {
    /// Crée un nouveau modèle d'erreur
    pub fn new(substitution_rate: f64, insertion_rate: f64, deletion_rate: f64) -> Self {
        Self {
            substitution_rate,
            insertion_rate,
            deletion_rate,
            seed: 42,
        }
    }

    /// Définit le seed
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Taux d'erreur total
    pub fn total_error_rate(&self) -> f64 {
        self.substitution_rate + self.insertion_rate + self.deletion_rate
    }

    /// Vérifie si le modèle est valide
    pub fn is_valid(&self) -> bool {
        self.total_error_rate() < 1.0
            && self.substitution_rate >= 0.0
            && self.insertion_rate >= 0.0
            && self.deletion_rate >= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_model() {
        let model = ErrorModel::default();
        assert_eq!(model.substitution_rate, 0.01);
        assert_eq!(model.insertion_rate, 0.005);
        assert_eq!(model.deletion_rate, 0.005);
    }

    #[test]
    fn test_total_error_rate() {
        let model = ErrorModel::default();
        assert!((model.total_error_rate() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn test_with_seed() {
        let model = ErrorModel::default().with_seed(123);
        assert_eq!(model.seed, 123);
    }

    #[test]
    fn test_validity() {
        let model = ErrorModel::default();
        assert!(model.is_valid());

        let invalid = ErrorModel {
            substitution_rate: 0.5,
            insertion_rate: 0.5,
            deletion_rate: 0.5,
            seed: 0,
        };
        assert!(!invalid.is_valid());
    }
}
