//! Simulation d'un canal de stockage ADN : synthèse, perte d'oligos, séquençage.
//!
//! Le modèle reproduit les étapes qui dégradent une archive entre son écriture et sa lecture :
//!
//! 1. **Perte** : chaque oligo est absent de la bibliothèque avec la probabilité `dropout`.
//! 2. **Synthèse** : les erreurs de synthèse frappent le modèle d'un oligo ; toutes ses lectures
//!    les portent, donc le consensus ne les efface pas.
//! 3. **Couverture** : le nombre de lectures d'un oligo suit une loi de Poisson de moyenne
//!    `coverage` ; un oligo sans lecture est perdu.
//! 4. **Séquençage** : chaque lecture reçoit ses propres substitutions, insertions et délétions.
//!
//! Les lectures sont produites dans un ordre aléatoire, sans information d'appoint : le décodage
//! doit donc se suffire des séquences.

use adn_core::codec::rng::SeededRng;
use adn_core::sequence::BASES;
use adn_core::{DnaError, Result};
use serde::{Deserialize, Serialize};

/// Taux d'erreur par nucléotide.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorRates {
    pub substitution: f64,
    pub insertion: f64,
    pub deletion: f64,
}

impl ErrorRates {
    /// Aucune erreur.
    pub const NONE: Self = Self {
        substitution: 0.0,
        insertion: 0.0,
        deletion: 0.0,
    };

    /// Taux donnés par nucléotide.
    pub const fn new(substitution: f64, insertion: f64, deletion: f64) -> Self {
        Self {
            substitution,
            insertion,
            deletion,
        }
    }

    /// Taux total.
    pub fn total(&self) -> f64 {
        self.substitution + self.insertion + self.deletion
    }

    fn validate(&self, name: &str) -> Result<()> {
        let valid = |rate: f64| rate.is_finite() && (0.0..1.0).contains(&rate);
        if valid(self.substitution)
            && valid(self.insertion)
            && valid(self.deletion)
            && self.total() < 1.0
        {
            Ok(())
        } else {
            Err(DnaError::InvalidConfig(format!(
                "taux d'erreur {name} invalides : substitution {}, insertion {}, délétion {}",
                self.substitution, self.insertion, self.deletion
            )))
        }
    }
}

/// Configuration du canal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Erreurs de synthèse, appliquées au modèle de chaque oligo.
    pub synthesis: ErrorRates,
    /// Erreurs de séquençage, appliquées à chaque lecture.
    pub sequencing: ErrorRates,
    /// Probabilité qu'un oligo soit absent de la bibliothèque.
    pub dropout: f64,
    /// Nombre moyen de lectures par oligo présent.
    pub coverage: f64,
    /// Graine du générateur : deux simulations avec la même graine sont identiques.
    pub seed: u64,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            synthesis: ErrorRates::NONE,
            sequencing: ErrorRates::new(0.005, 0.0, 0.0),
            dropout: 0.0,
            coverage: 1.0,
            seed: 42,
        }
    }
}

impl ChannelConfig {
    /// Vérifie la cohérence de la configuration.
    pub fn validate(&self) -> Result<()> {
        self.synthesis.validate("de synthèse")?;
        self.sequencing.validate("de séquençage")?;
        if !self.dropout.is_finite() || !(0.0..=1.0).contains(&self.dropout) {
            return Err(DnaError::InvalidConfig(format!(
                "taux de perte invalide : {}",
                self.dropout
            )));
        }
        if !self.coverage.is_finite() || !(0.0..=100_000.0).contains(&self.coverage) {
            return Err(DnaError::InvalidConfig(format!(
                "couverture invalide : {}",
                self.coverage
            )));
        }
        Ok(())
    }
}

/// Nombre de chaque type d'erreur appliqué à une séquence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorCounts {
    pub substitutions: u32,
    pub insertions: u32,
    pub deletions: u32,
}

impl ErrorCounts {
    /// Nombre total d'erreurs.
    pub fn total(&self) -> u32 {
        self.substitutions + self.insertions + self.deletions
    }
}

/// Une lecture simulée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimulatedRead {
    /// Rang de l'oligo d'origine dans l'archive.
    pub oligo: u32,
    pub sequence: Vec<u8>,
    /// Erreurs de séquençage propres à cette lecture.
    pub errors: ErrorCounts,
}

/// Bilan d'une simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationStats {
    pub oligos: u32,
    /// Oligos absents : perdus, ou sans aucune lecture.
    pub lost_oligos: u32,
    pub reads: usize,
    /// Lectures dont la longueur n'est plus celle d'un oligo (insertions ou délétions).
    pub reads_wrong_length: usize,
    /// Lectures sans aucune erreur, ni de synthèse ni de séquençage.
    pub reads_perfect: usize,
    pub synthesis_errors: ErrorCounts,
    pub sequencing_errors: ErrorCounts,
    pub copies_min: u32,
    pub copies_max: u32,
}

const STREAM_DROPOUT: u64 = 0x0000_0000_0000_0021;
const STREAM_SYNTHESIS: u64 = 0x0000_0000_0000_0022;
const STREAM_POISSON: u64 = 0x0000_0000_0000_0023;
const STREAM_SEQUENCING: u64 = 0x0000_0000_0000_0024;
const STREAM_SHUFFLE: u64 = 0x0000_0000_0000_0025;

#[inline]
fn rank_of(base: u8) -> u64 {
    match base {
        b'A' => 0,
        b'C' => 1,
        b'G' => 2,
        _ => 3,
    }
}

/// Applique des erreurs à une séquence : retourne le résultat et le nombre d'erreurs de chaque
/// type. Une substitution remplace la base par une autre.
pub fn apply_errors(
    sequence: &[u8],
    rates: &ErrorRates,
    rng: &mut SeededRng,
) -> (Vec<u8>, ErrorCounts) {
    let mut out = Vec::with_capacity(sequence.len() + 4);
    let mut counts = ErrorCounts::default();
    let total = rates.total();
    for &base in sequence {
        let roll = rng.next_f64();
        if roll < rates.substitution {
            let offset = 1 + rng.below(3);
            out.push(BASES[((rank_of(base) + offset) & 3) as usize]);
            counts.substitutions += 1;
        } else if roll < rates.substitution + rates.insertion {
            out.push(BASES[rng.below(4) as usize]);
            out.push(base);
            counts.insertions += 1;
        } else if roll < total {
            counts.deletions += 1;
        } else {
            out.push(base);
        }
    }
    (out, counts)
}

/// Nombre de tirages d'une loi de Poisson de moyenne `mean`.
fn poisson(mean: f64, rng: &mut SeededRng) -> u32 {
    if mean <= 0.0 {
        return 0;
    }
    if mean < 60.0 {
        // Méthode de Knuth.
        let limit = (-mean).exp();
        let mut product = 1.0;
        let mut count = 0u32;
        loop {
            product *= rng.next_f64().max(f64::MIN_POSITIVE);
            if product <= limit {
                return count;
            }
            count += 1;
        }
    }
    // Au-delà, approximation normale (transformation de Box-Muller).
    let u1 = rng.next_f64().max(f64::MIN_POSITIVE);
    let u2 = rng.next_f64();
    let normal = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    (mean + normal * mean.sqrt())
        .round()
        .clamp(0.0, f64::from(u32::MAX)) as u32
}

/// Canal de transmission simulé. Il est réutilisable : chaque appel à [`DnaChannel::simulate`]
/// donne les mêmes lectures pour les mêmes oligos.
pub struct DnaChannel {
    config: ChannelConfig,
}

impl DnaChannel {
    /// Crée un canal ; la configuration est vérifiée.
    pub fn new(config: ChannelConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Configuration du canal.
    pub fn config(&self) -> &ChannelConfig {
        &self.config
    }

    /// Simule la lecture d'une archive et retourne les lectures, dans un ordre aléatoire.
    ///
    /// Chaque oligo tire de ses propres flux, dérivés de son rang : le résultat pour un oligo ne
    /// dépend ni de l'ordre ni du nombre des autres.
    pub fn simulate(&self, oligos: &[Vec<u8>]) -> (Vec<SimulatedRead>, SimulationStats) {
        let mut stats = SimulationStats {
            oligos: oligos.len() as u32,
            copies_min: u32::MAX,
            ..SimulationStats::default()
        };
        let mut reads: Vec<SimulatedRead> = Vec::new();
        for (index, oligo) in oligos.iter().enumerate() {
            let key = index as u64 ^ self.config.seed;
            let dropped = self.config.dropout > 0.0
                && SeededRng::new(key, STREAM_DROPOUT).next_f64() < self.config.dropout;
            let (template, synthesis_errors) = if self.config.synthesis.total() > 0.0 {
                apply_errors(
                    oligo,
                    &self.config.synthesis,
                    &mut SeededRng::new(key, STREAM_SYNTHESIS),
                )
            } else {
                (oligo.clone(), ErrorCounts::default())
            };
            stats.synthesis_errors.substitutions += synthesis_errors.substitutions;
            stats.synthesis_errors.insertions += synthesis_errors.insertions;
            stats.synthesis_errors.deletions += synthesis_errors.deletions;
            let copies = if dropped {
                0
            } else {
                poisson(
                    self.config.coverage,
                    &mut SeededRng::new(key, STREAM_POISSON),
                )
            };
            if copies == 0 {
                stats.lost_oligos += 1;
                continue;
            }
            stats.copies_min = stats.copies_min.min(copies);
            stats.copies_max = stats.copies_max.max(copies);

            let mut rng = SeededRng::new(key, STREAM_SEQUENCING);
            for _ in 0..copies {
                let (sequence, errors) = if self.config.sequencing.total() > 0.0 {
                    apply_errors(&template, &self.config.sequencing, &mut rng)
                } else {
                    (template.clone(), ErrorCounts::default())
                };
                stats.reads_wrong_length += usize::from(sequence.len() != oligo.len());
                stats.reads_perfect +=
                    usize::from(errors.total() == 0 && synthesis_errors.total() == 0);
                stats.sequencing_errors.substitutions += errors.substitutions;
                stats.sequencing_errors.insertions += errors.insertions;
                stats.sequencing_errors.deletions += errors.deletions;
                reads.push(SimulatedRead {
                    oligo: index as u32,
                    sequence,
                    errors,
                });
            }
        }
        if stats.copies_min == u32::MAX {
            stats.copies_min = 0;
        }
        stats.reads = reads.len();

        let mut rng = SeededRng::new(self.config.seed, STREAM_SHUFFLE);
        for i in (1..reads.len()).rev() {
            reads.swap(i, rng.below(i as u64 + 1) as usize);
        }
        (reads, stats)
    }

    /// Altère une seule séquence avec les erreurs de séquençage.
    pub fn transmit(&self, sequence: &[u8]) -> (Vec<u8>, ErrorCounts) {
        let mut rng = SeededRng::new(self.config.seed, STREAM_SEQUENCING);
        apply_errors(sequence, &self.config.sequencing, &mut rng)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oligos(count: usize, length: usize) -> Vec<Vec<u8>> {
        let mut rng = SeededRng::new(5, 6);
        (0..count)
            .map(|_| {
                let mut out = vec![0u8; length];
                rng.fill_bytes(&mut out);
                out.iter_mut()
                    .for_each(|base| *base = BASES[(*base & 3) as usize]);
                out
            })
            .collect()
    }

    #[test]
    fn config_is_validated() {
        assert!(DnaChannel::new(ChannelConfig::default()).is_ok());
        let bad = ChannelConfig {
            sequencing: ErrorRates::new(0.9, 0.9, 0.0),
            ..ChannelConfig::default()
        };
        assert!(DnaChannel::new(bad).is_err());
        let bad = ChannelConfig {
            dropout: 1.5,
            ..ChannelConfig::default()
        };
        assert!(DnaChannel::new(bad).is_err());
        let bad = ChannelConfig {
            coverage: -1.0,
            ..ChannelConfig::default()
        };
        assert!(DnaChannel::new(bad).is_err());
        let bad = ChannelConfig {
            synthesis: ErrorRates::new(f64::NAN, 0.0, 0.0),
            ..ChannelConfig::default()
        };
        assert!(DnaChannel::new(bad).is_err());
    }

    #[test]
    fn perfect_channel_returns_the_archive() {
        let pool = oligos(50, 160);
        let config = ChannelConfig {
            sequencing: ErrorRates::NONE,
            coverage: 3.0,
            ..ChannelConfig::default()
        };
        let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&pool);
        // La couverture est une moyenne : le nombre de lectures de chaque oligo suit une loi
        // de Poisson de paramètre 3. Un oligo sur vingt n'en reçoit aucune (e^-3), et s'en
        // trouve perdu ; c'est le comportement attendu d'une faible couverture.
        assert_eq!(stats.reads, reads.len());
        assert!((100..=200).contains(&stats.reads), "{}", stats.reads);
        assert!(stats.lost_oligos <= 10, "{}", stats.lost_oligos);
        assert_eq!(
            stats.lost_oligos,
            50 - reads
                .iter()
                .map(|r| r.oligo)
                .collect::<std::collections::HashSet<_>>()
                .len() as u32
        );
        assert!(stats.copies_min == 1 || stats.copies_min == 0);
        assert!(stats.copies_max >= 3);
        assert_eq!(stats.reads_perfect, stats.reads);
        for read in &reads {
            assert_eq!(read.sequence, pool[read.oligo as usize]);
        }
        assert_eq!(stats.reads_wrong_length, 0);

        // Couverture profonde : plus aucun oligo sans lecture.
        let config = ChannelConfig {
            sequencing: ErrorRates::NONE,
            coverage: 60.0,
            ..ChannelConfig::default()
        };
        let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&pool);
        assert_eq!(stats.lost_oligos, 0);
        assert!(stats.copies_min >= 20, "{}", stats.copies_min);
        for read in &reads {
            assert_eq!(read.sequence, pool[read.oligo as usize]);
        }
    }

    #[test]
    fn same_seed_gives_same_reads_and_reads_are_shuffled() {
        let pool = oligos(80, 152);
        let config = ChannelConfig {
            synthesis: ErrorRates::new(0.0, 0.0, 0.002),
            sequencing: ErrorRates::new(0.01, 0.002, 0.003),
            dropout: 0.1,
            coverage: 4.0,
            seed: 7,
        };
        let (first, stats_first) = DnaChannel::new(config).unwrap().simulate(&pool);
        let (second, stats_second) = DnaChannel::new(config).unwrap().simulate(&pool);
        assert_eq!(first, second);
        assert_eq!(stats_first, stats_second);
        assert!(stats_first.lost_oligos > 0);
        assert!(stats_first.reads_wrong_length > 0);
        assert!(first.windows(2).any(|pair| pair[0].oligo > pair[1].oligo));
    }

    #[test]
    fn measured_rates_match_configured_rates() {
        let sequence = oligos(1, 400_000).remove(0);
        let rates = ErrorRates::new(0.01, 0.003, 0.002);
        let (out, counts) = apply_errors(&sequence, &rates, &mut SeededRng::new(3, 1));
        for (measured, expected) in [
            (counts.substitutions as f64 / 400_000.0, 0.01),
            (counts.insertions as f64 / 400_000.0, 0.003),
            (counts.deletions as f64 / 400_000.0, 0.002),
        ] {
            assert!(
                (measured - expected).abs() < 0.001,
                "{measured} vs {expected}"
            );
        }
        let expected_len = 400_000 + counts.insertions as usize - counts.deletions as usize;
        assert_eq!(out.len(), expected_len);
    }

    #[test]
    fn substitution_changes_the_base() {
        let original = b"ACGTACGTACGT";
        let (out, counts) = apply_errors(
            original,
            &ErrorRates::new(1.0, 0.0, 0.0),
            &mut SeededRng::new(1, 1),
        );
        assert_eq!(counts.substitutions, 12);
        assert_eq!(counts.total(), 12);
        assert_eq!(out.len(), original.len());
        for (&before, &after) in original.iter().zip(&out) {
            assert_ne!(before, after);
            assert!(BASES.contains(&after));
        }
    }

    #[test]
    fn deletions_only_shorten_and_insertions_only_lengthen() {
        let original = b"ACGTTGCAACGTTCGA";
        let (out, counts) = apply_errors(
            original,
            &ErrorRates::new(0.0, 1.0, 0.0),
            &mut SeededRng::new(2, 1),
        );
        assert_eq!(out.len(), 32);
        assert_eq!(counts.insertions, 16);
        let (out, counts) = apply_errors(
            original,
            &ErrorRates::new(0.0, 0.0, 1.0),
            &mut SeededRng::new(2, 1),
        );
        assert!(out.is_empty());
        assert_eq!(counts.deletions, 16);
        let (out, counts) = apply_errors(original, &ErrorRates::NONE, &mut SeededRng::new(2, 1));
        assert_eq!(out, original);
        assert_eq!(counts.total(), 0);
    }

    #[test]
    fn poisson_matches_its_mean_over_a_range() {
        for mean in [0.0f64, 0.5, 3.0, 40.0, 500.0] {
            let mut rng = SeededRng::new(11, 12);
            let trials = 20_000u32;
            let total: u32 = (0..trials).map(|_| poisson(mean, &mut rng)).sum();
            let measured = total as f64 / f64::from(trials);
            assert!(
                (measured - mean).abs() < 0.05 * mean.max(0.2),
                "mean={mean}: {measured}"
            );
        }
    }

    #[test]
    fn synthesis_errors_are_shared_by_all_reads_of_an_oligo() {
        let pool = oligos(200, 160);
        // Sans erreur de séquençage : les lectures d'un oligo ne diffèrent du modèle que par
        // les erreurs de synthèse, identiques pour toutes.
        let config = ChannelConfig {
            synthesis: ErrorRates::new(0.02, 0.0, 0.0),
            sequencing: ErrorRates::NONE,
            coverage: 5.0,
            ..ChannelConfig::default()
        };
        let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&pool);
        // 200 oligos, 5 lectures en moyenne : environ 1 000 lectures.
        assert!((700..=1_300).contains(&stats.reads), "{}", stats.reads);
        assert!(stats.synthesis_errors.substitutions > 300);
        let damaged_oligos = reads
            .iter()
            .filter(|read| read.sequence != pool[read.oligo as usize])
            .map(|read| read.oligo)
            .collect::<std::collections::HashSet<_>>();
        assert!(damaged_oligos.len() >= 25, "{}", damaged_oligos.len());
        for oligo in &damaged_oligos {
            let sequences: std::collections::HashSet<&Vec<u8>> = reads
                .iter()
                .filter(|read| read.oligo == *oligo)
                .map(|read| &read.sequence)
                .collect();
            assert_eq!(
                sequences.len(),
                1,
                "toutes les lectures de l'oligo {oligo} sont identiques"
            );
        }
        let damaged = reads
            .iter()
            .filter(|read| read.sequence != pool[read.oligo as usize])
            .count();
        assert_eq!(stats.reads_perfect + damaged, stats.reads);

        // Avec erreurs de séquençage, chaque lecture porte en plus les siennes : les lectures
        // d'un même oligo diffèrent entre elles.
        let config = ChannelConfig {
            synthesis: ErrorRates::new(0.02, 0.0, 0.0),
            sequencing: ErrorRates::new(0.02, 0.0, 0.0),
            coverage: 5.0,
            ..ChannelConfig::default()
        };
        let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&pool);
        // Environ 1 000 lectures de 160 nt à 2 % : près de 3 200 substitutions.
        assert!(stats.sequencing_errors.substitutions > 2_000);
        let oligo = reads[0].oligo;
        let sequences: std::collections::HashSet<&Vec<u8>> = reads
            .iter()
            .filter(|read| read.oligo == oligo)
            .map(|read| &read.sequence)
            .collect();
        assert!(sequences.len() > 1, "les lectures d'un oligo diffèrent");
    }

    #[test]
    fn transmit_uses_sequencing_errors_only() {
        let config = ChannelConfig {
            synthesis: ErrorRates::new(0.5, 0.3, 0.1),
            sequencing: ErrorRates::NONE,
            ..ChannelConfig::default()
        };
        let channel = DnaChannel::new(config).unwrap();
        let (out, counts) = channel.transmit(b"ACGT");
        assert_eq!(out, b"ACGT");
        assert_eq!(counts.total(), 0);
    }
}
