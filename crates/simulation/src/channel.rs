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
            temperature: 25.0, // 25°C
            ph: 7.0,           // pH neutre
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
    ///
    /// Parcourt les bases originales et construit une nouvelle séquence :
    /// ne jamais muter un vecteur en cours d'itération (les insertions /
    /// délétions décalent les indices suivants — et un remove(i) sur un
    /// vecteur raccourci par des délétions précédentes entrait hors bornes).
    pub fn transmit(&mut self, sequence: &DnaSequence) -> Result<(DnaSequence, SimulationMetrics)> {
        let mut corrupted = sequence.clone();
        let mut metrics = SimulationMetrics::new();

        // Extraire les taux avant les appels mutables
        let sub_rate = self.config.error_model.substitution_rate;
        let ins_rate = self.config.error_model.insertion_rate;
        let del_rate = self.config.error_model.deletion_rate;
        let total_rate = sub_rate + ins_rate + del_rate;

        let mut new_bases = Vec::with_capacity(sequence.bases.len());

        for &base in &sequence.bases {
            let roll: f64 = self.rng.gen();

            if roll < sub_rate {
                // Substitution
                new_bases.push(self.substitute_base(base));
                metrics.substitutions += 1;
            } else if roll < sub_rate + ins_rate {
                // Insertion d'une base aléatoire avant la base courante
                new_bases.push(self.random_base());
                new_bases.push(base);
                metrics.insertions += 1;
            } else if roll < total_rate {
                // Délétion : la base n'est pas recopiée
                metrics.deletions += 1;
            } else {
                // Transmission fidèle
                new_bases.push(base);
            }
        }

        corrupted.bases = new_bases;

        // Recalculer les métadonnées (GC, homopolymer, entropie) : celles
        // clonées décrivaient la séquence d'origine, pas la séquence corrompue.
        corrupted.metadata = adn_core::SequenceMetadata::compute(
            &corrupted.bases,
            corrupted.metadata.original_file.clone(),
            corrupted.metadata.chunk_index,
            corrupted.metadata.chunk_size,
            corrupted.metadata.seed,
            corrupted.metadata.encoding_scheme.clone(),
        );

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
    pub fn transmit_iterations(
        &mut self,
        sequence: &DnaSequence,
        n: usize,
    ) -> Vec<Result<(DnaSequence, SimulationMetrics)>> {
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

    #[test]
    fn test_deletion_heavy_no_panic_and_consistent_length() {
        // Régression : l'ancienne implémentation mutait `corrupted.bases`
        // avec remove(i) tout en itérant sur les indices originaux → panic
        // (index hors bornes) dès que les délétions dépassaient les insertions.
        for seed in [0u64, 1, 42, 999] {
            let mut config = ChannelConfig::default();
            config.error_model.substitution_rate = 0.0;
            config.error_model.insertion_rate = 0.0;
            config.error_model.deletion_rate = 0.9;
            config.error_model.seed = seed;

            let mut channel = DnaChannel::new(config);

            let bases = vec![IupacBase::A; 1000];
            let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 1000, 42);

            let (corrupted, metrics) = channel.transmit(&seq).unwrap();

            // Longueur exacte : original - délétions (aucune insertion/substitution)
            assert_eq!(
                corrupted.bases.len(),
                1000 - metrics.deletions,
                "seed={}: longueur incohérente",
                seed
            );
            assert_eq!(metrics.substitutions, 0);
            assert_eq!(metrics.insertions, 0);
        }
    }

    #[test]
    fn test_insertion_only_length() {
        let mut config = ChannelConfig::default();
        config.error_model.substitution_rate = 0.0;
        config.error_model.insertion_rate = 1.0; // insère devant chaque base
        config.error_model.deletion_rate = 0.0;
        config.error_model.seed = 7;

        let mut channel = DnaChannel::new(config);

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let (corrupted, metrics) = channel.transmit(&seq).unwrap();

        assert_eq!(metrics.insertions, 4);
        assert_eq!(corrupted.bases.len(), 8);

        // Les bases originales doivent rester présentes, dans l'ordre,
        // en positions impaires (une base insérée devant chacune)
        let originals: Vec<IupacBase> =
            corrupted.bases.iter().skip(1).step_by(2).copied().collect();
        assert_eq!(originals, seq.bases);
    }

    #[test]
    fn test_metadata_recomputed_after_corruption() {
        // Les métadonnées de la séquence corrompue doivent décrire la
        // séquence corrompue (pas un clone des stats de l'originale).
        let mut config = ChannelConfig::default();
        config.error_model.substitution_rate = 1.0; // toutes les bases changent
        config.error_model.insertion_rate = 0.0;
        config.error_model.deletion_rate = 0.0;
        config.error_model.seed = 3;

        let mut channel = DnaChannel::new(config);

        let bases = vec![IupacBase::A, IupacBase::A, IupacBase::A, IupacBase::A];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let (corrupted, _metrics) = channel.transmit(&seq).unwrap();

        // A => pas A : GC nécessairement > 0 alors que l'originale était 0%
        assert!(corrupted.metadata.gc_ratio > 0.0);
        assert!(!corrupted.bases.contains(&IupacBase::A));
    }

    #[test]
    fn test_substitution_changes_base() {
        let mut config = ChannelConfig::default();
        config.error_model.substitution_rate = 1.0;
        config.error_model.insertion_rate = 0.0;
        config.error_model.deletion_rate = 0.0;
        config.error_model.seed = 5;

        let mut channel = DnaChannel::new(config);

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let (corrupted, metrics) = channel.transmit(&seq).unwrap();

        assert_eq!(metrics.substitutions, 4);
        // Même longueur, aucune base inchangée
        assert_eq!(corrupted.bases.len(), 4);
        for (orig, new) in seq.bases.iter().zip(&corrupted.bases) {
            assert_ne!(orig, new);
        }
    }
}
