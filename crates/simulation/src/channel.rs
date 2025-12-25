//! Canal de transmission ADN simulé

use crate::error_model::ErrorModel;
use crate::metrics::SimulationMetrics;
use adn_core::{DnaSequence, IupacBase, Result};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

/// Configuration du canal ADN
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Modèle d'erreur
    pub error_model: ErrorModel,

    /// Température (affecte les taux d'erreur)
    pub temperature: f64,

    /// pH (affecte les taux d'erreur)
    pub ph: f64,

    /// Durée de stockage en jours
    pub storage_duration_days: u32,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            error_model: ErrorModel::default(),
            temperature: 25.0,  // 25°C
            ph: 7.0,            // pH neutre
            storage_duration_days: 30,
        }
    }
}

/// Canal de transmission ADN simulé
pub struct DnaChannel {
    config: ChannelConfig,
    rng: ChaCha8Rng,
}

impl DnaChannel {
    /// Crée un nouveau canal
    pub fn new(config: ChannelConfig) -> Self {
        let seed = config.error_model.seed;
        Self {
            config,
            rng: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// Simule la transmission avec erreurs
    pub fn transmit(&mut self, sequence: &DnaSequence) -> Result<(DnaSequence, SimulationMetrics)> {
        let mut corrupted = sequence.clone();
        let mut metrics = SimulationMetrics::new();

        // Extraire les taux avant les appels mutables
        let sub_rate = self.config.error_model.substitution_rate;
        let ins_rate = self.config.error_model.insertion_rate;
        let del_rate = self.config.error_model.deletion_rate;
        let total_rate = sub_rate + ins_rate + del_rate;

        for (i, base) in sequence.bases.iter().enumerate() {
            let roll: f64 = self.rng.gen();

            if roll < sub_rate {
                // Substitution
                let new_base = self.substitute_base(*base);
                corrupted.bases[i] = new_base;
                metrics.substitutions += 1;
            } else if roll < sub_rate + ins_rate {
                // Insertion
                let new_base = self.random_base();
                corrupted.bases.insert(i, new_base);
                metrics.insertions += 1;
            } else if roll < total_rate {
                // Délétion
                corrupted.bases.remove(i);
                metrics.deletions += 1;
            }
        }

        metrics.total_bases = sequence.bases.len();
        metrics.affected_bases = metrics.substitutions + metrics.insertions + metrics.deletions;

        Ok((corrupted, metrics))
    }

    /// Substitue une base par une autre
    fn substitute_base(&mut self, base: IupacBase) -> IupacBase {
        let bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        // Choisir une base différente
        let mut new_base = base;
        while new_base == base {
            new_base = bases[self.rng.gen_range(0..4)];
        }

        new_base
    }

    /// Génère une base aléatoire
    fn random_base(&mut self) -> IupacBase {
        match self.rng.gen_range(0..4) {
            0 => IupacBase::A,
            1 => IupacBase::C,
            2 => IupacBase::G,
            _ => IupacBase::T,
        }
    }

    /// Simule plusieurs itérations
    pub fn transmit_iterations(&mut self, sequence: &DnaSequence, n: usize) -> Vec<Result<(DnaSequence, SimulationMetrics)>> {
        (0..n).map(|_| self.transmit(sequence)).collect()
    }

    /// Réinitialise le RNG
    pub fn reset_rng(&mut self) {
        self.rng = ChaCha8Rng::seed_from_u64(self.config.error_model.seed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_creation() {
        let config = ChannelConfig::default();
        let _channel = DnaChannel::new(config);
        // Juste vérifier que ça compile
    }

    #[test]
    fn test_transmit() {
        let config = ChannelConfig::default();
        let mut channel = DnaChannel::new(config);

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let (_corrupted, metrics) = channel.transmit(&seq).unwrap();

        assert_eq!(metrics.total_bases, 4);
    }

    #[test]
    fn test_high_error_rate() {
        let mut config = ChannelConfig::default();
        config.error_model.substitution_rate = 0.5;
        config.error_model.seed = 123;

        let mut channel = DnaChannel::new(config);

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let (_corrupted, _metrics) = channel.transmit(&seq).unwrap();
    }
}
