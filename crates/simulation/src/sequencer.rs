//! Séquenceur Illumina simulé : lectures appariées et lectures d'index d'un pool.
//!
//! Chaque molécule du pool donne un amas, lu un nombre de fois qui suit une loi de Poisson de
//! moyenne `coverage`. De chaque amas, le séquenceur tire :
//!
//! - la **lecture 1**, amorcée au site de la lecture 1 : depuis la première base de l'insert,
//!   vers l'extrémité P7. Assez longue, elle traverse l'insert et lit l'adaptateur qui le suit.
//! - la **lecture 2**, amorcée au site de la lecture 2 : elle commence par l'extrémité mosaïque,
//!   lit l'insert à l'envers, puis, assez longue, l'adaptateur qui le précède.
//! - les **lectures d'index** : i7, que l'amorce P7 lit complément inversé, puis i5, que l'amorce
//!   P5 lit dans le sens de la molécule.
//!
//! Une lecture s'arrête au bout de la molécule. Les erreurs de séquençage sont propres à chaque
//! lecture : leur probabilité croît du début à la fin, d'un sixième de la moyenne à près de deux
//! fois, et la qualité Phred inscrite à chaque base est celle de sa probabilité d'erreur.
//!
//! Avec `index_hopping`, une fraction des paires porte les index d'une autre molécule du pool.
//! Le phénomène, observé sur les cellules à motif, range la paire dans un autre échantillon :
//! c'est une raison de plus d'exiger des index distants, et de contrôler l'intégrité de chaque
//! archive.

use adn_core::codec::rng::SeededRng;
use adn_core::library::{Library, PoolMolecule, MOSAIC_END, P5, P7};
use adn_core::sequence::{reverse_complement, BASES};
use adn_core::{DnaError, Result};
use serde::{Deserialize, Serialize};

use crate::{
    apply_bursts, apply_errors, poisson, BurstConfig, ErrorCounts, ErrorRates, STREAM_DROPOUT,
    STREAM_POISSON, STREAM_SEQUENCING, STREAM_SHUFFLE, STREAM_SYNTHESIS,
};

const STREAM_INDEX: u64 = 0x0000_0000_0000_0026;
const STREAM_HOPPING: u64 = 0x0000_0000_0000_0027;

/// Plus forte qualité inscrite.
pub const MAX_PHRED: u8 = 40;

/// Configuration du séquenceur.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SequencerConfig {
    /// Longueur des lectures 1 et 2, en bases.
    pub read_length: usize,
    /// Longueur des lectures d'index ; par défaut, celle des index de la bibliothèque.
    pub index_read_length: Option<usize>,
    /// Erreurs de séquençage des lectures 1 et 2, par nucléotide en moyenne.
    pub sequencing: ErrorRates,
    /// Erreurs de séquençage en salves, dans les lectures 1 et 2.
    pub bursts: BurstConfig,
    /// Erreurs de séquençage des lectures d'index, uniformes.
    pub index_sequencing: ErrorRates,
    /// Erreurs de synthèse, appliquées à l'insert de chaque molécule.
    pub synthesis: ErrorRates,
    /// Probabilité qu'une molécule soit absente du pool.
    pub dropout: f64,
    /// Nombre moyen de paires de lectures par molécule présente.
    pub coverage: f64,
    /// Fraction des paires de lectures dont les index sont ceux d'une autre molécule.
    pub index_hopping: f64,
    /// Graine du générateur : deux simulations avec la même graine sont identiques.
    pub seed: u64,
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            read_length: 300,
            index_read_length: None,
            sequencing: ErrorRates::new(0.001, 0.0, 0.0),
            bursts: BurstConfig::NONE,
            index_sequencing: ErrorRates::new(0.001, 0.0, 0.0),
            synthesis: ErrorRates::NONE,
            dropout: 0.0,
            coverage: 1.0,
            index_hopping: 0.0,
            seed: 42,
        }
    }
}

impl SequencerConfig {
    /// Vérifie la cohérence de la configuration.
    pub fn validate(&self) -> Result<()> {
        if !(1..=10_000).contains(&self.read_length) {
            return Err(DnaError::InvalidConfig(format!(
                "longueur de lecture invalide : {}",
                self.read_length
            )));
        }
        if let Some(length) = self.index_read_length {
            if !(1..=100).contains(&length) {
                return Err(DnaError::InvalidConfig(format!(
                    "longueur des lectures d'index invalide : {length}"
                )));
            }
        }
        self.sequencing.validate("de séquençage")?;
        self.bursts.validate()?;
        self.index_sequencing.validate("de séquençage des index")?;
        self.synthesis.validate("de synthèse")?;
        if self.sequencing.total() * RAMP_END >= 1.0 {
            return Err(DnaError::InvalidConfig(format!(
                "taux d'erreur de séquençage trop élevés en fin de lecture : {}",
                self.sequencing.total()
            )));
        }
        let fraction = |name: &str, value: f64, max: f64| {
            if value.is_finite() && (0.0..=max).contains(&value) {
                Ok(())
            } else {
                Err(DnaError::InvalidConfig(format!(
                    "{name} invalide : {value}"
                )))
            }
        };
        fraction("taux de perte", self.dropout, 1.0)?;
        fraction("taux de sauts d'index", self.index_hopping, 1.0)?;
        fraction("couverture", self.coverage, 100_000.0)?;
        Ok(())
    }
}

/// Une paire de lectures, avec ses lectures d'index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadPair {
    /// Rang de la molécule lue, dans le pool.
    pub molecule: u32,
    pub r1: Vec<u8>,
    /// Qualité Phred+33 de la lecture 1, un caractère par base.
    pub quality_r1: Vec<u8>,
    pub r2: Vec<u8>,
    pub quality_r2: Vec<u8>,
    /// Lecture d'index i7 : l'index de la molécule, complément inversé.
    pub i7: Vec<u8>,
    pub quality_i7: Vec<u8>,
    /// Lecture d'index i5 ; absente d'une bibliothèque à un seul index.
    pub i5: Option<Vec<u8>>,
    pub quality_i5: Option<Vec<u8>>,
    /// Molécule dont les index ont été lus à la place des siens (saut d'index).
    pub indexes_from: Option<u32>,
    /// Erreurs de séquençage des lectures 1 et 2.
    pub errors: ErrorCounts,
    /// Erreurs de séquençage des lectures d'index.
    pub index_errors: ErrorCounts,
}

/// Bilan d'un séquençage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequencerStats {
    pub molecules: usize,
    /// Molécules absentes, ou sans aucune lecture.
    pub lost_molecules: usize,
    pub pairs: usize,
    /// Paires dont les lectures 1 et 2 sont sans erreur de séquençage.
    pub pairs_perfect: usize,
    /// Paires dont les index sont ceux d'une autre molécule.
    pub hopped: usize,
    pub sequencing_errors: ErrorCounts,
    pub index_errors: ErrorCounts,
    pub synthesis_errors: ErrorCounts,
    pub copies_min: u32,
    pub copies_max: u32,
}

/// Identifiant d'une paire de lectures, au format des séquenceurs : unique par paire.
pub fn read_id(pair: usize) -> String {
    format!("SIM:1:FC:1:1101:{}:{}", pair / 1000, pair % 1000)
}

/// En-tête FASTQ de la lecture `read` d'une paire : 1 et 2 pour les lectures, 3 et 4 pour les
/// index. Les index y figurent tels qu'ils ont été lus. L'échantillon d'origine, quand il est
/// donné, suit sous la forme `sample=<nom>` : [`truth_sample`] le relit.
pub fn read_header(id: &str, read: u8, pair: &ReadPair, sample: Option<&str>) -> String {
    let text = |bases: &[u8]| bases.iter().map(|&base| base as char).collect::<String>();
    let indexes = match &pair.i5 {
        Some(i5) => format!("{}+{}", text(&pair.i7), text(i5)),
        None => text(&pair.i7),
    };
    let mut header = format!("{id} {read}:N:0:{indexes}");
    if let Some(sample) = sample {
        header.push_str(&format!(" sample={sample}"));
    }
    header
}

/// L'échantillon d'origine inscrit dans un en-tête par [`read_header`], s'il y est.
pub fn truth_sample(header: &str) -> Option<&str> {
    header
        .split_whitespace()
        .find_map(|token| token.strip_prefix("sample="))
}

/// Qualité Phred d'une probabilité d'erreur, bornée à [`MAX_PHRED`].
fn phred(probability: f64) -> u8 {
    if probability <= 1e-4 {
        MAX_PHRED
    } else {
        (-10.0 * probability.log10())
            .round()
            .clamp(2.0, f64::from(MAX_PHRED)) as u8
    }
}

/// Erreur relative en fin de lecture, devant sa moyenne.
const RAMP_END: f64 = 11.0 / 6.0;

/// Erreur relative à la position `position` d'une lecture de `length` bases : un sixième de la
/// moyenne au début, [`RAMP_END`] à la fin, la moyenne sur la lecture.
fn ramp(position: usize, length: usize) -> f64 {
    let span = length.saturating_sub(1).max(1);
    (2.0 * position as f64 / span as f64 + 0.2) / 1.2
}

#[inline]
fn rank_of(base: u8) -> u64 {
    match base {
        b'A' => 0,
        b'C' => 1,
        b'G' => 2,
        _ => 3,
    }
}

/// Lit un gabarit : la lecture, sa qualité base à base, et le compte de ses erreurs. Les taux
/// sont multipliés par la rampe quand `ramped` est vrai.
fn sequence_read(
    template: &[u8],
    rates: &ErrorRates,
    ramped: bool,
    bursts: &BurstConfig,
    rng: &mut SeededRng,
) -> (Vec<u8>, Vec<u8>, ErrorCounts) {
    if rates.total() == 0.0 && bursts.rate <= 0.0 {
        return (
            template.to_vec(),
            vec![MAX_PHRED; template.len()],
            ErrorCounts::default(),
        );
    }
    let mut read = Vec::with_capacity(template.len());
    let mut quality = Vec::with_capacity(template.len());
    let mut counts = ErrorCounts::default();
    for (position, &base) in template.iter().enumerate() {
        let factor = if ramped {
            ramp(position, template.len())
        } else {
            1.0
        };
        let substitution = rates.substitution * factor;
        let insertion = rates.insertion * factor;
        let total = rates.total() * factor;
        let score = phred(substitution);
        let roll = rng.next_f64();
        if roll < substitution {
            let offset = 1 + rng.below(3);
            read.push(BASES[((rank_of(base) + offset) & 3) as usize]);
            quality.push(score);
            counts.substitutions += 1;
        } else if roll < substitution + insertion {
            read.push(BASES[rng.below(4) as usize]);
            read.push(base);
            quality.push(score);
            quality.push(score);
            counts.insertions += 1;
        } else if roll < total {
            counts.deletions += 1;
        } else {
            read.push(base);
            quality.push(score);
        }
    }
    // Les salves frappent la lecture après coup : la qualité de chaque base reste celle du
    // début, plus élevée qu'une salve ne le dirait.
    let (read, burst_counts) = apply_bursts(&read, bursts, rng);
    counts += burst_counts;
    (read, quality, counts)
}

/// Les lectures d'index d'une molécule, sans erreur.
struct IndexTemplates {
    /// Index i7, complément inversé : l'amorce P7 lit l'autre brin.
    i7: Vec<u8>,
    /// Index i5, dans le sens de la molécule : l'amorce P5 lit le même brin.
    i5: Option<Vec<u8>>,
}

fn index_templates(molecule: &PoolMolecule, length: usize, with_i5: bool) -> IndexTemplates {
    let sequence = &molecule.sequence;
    let i7_end = sequence.len() - P7.len();
    IndexTemplates {
        i7: reverse_complement(&sequence[i7_end.saturating_sub(length)..i7_end]),
        i5: with_i5.then(|| sequence[P5.len()..(P5.len() + length).min(sequence.len())].to_vec()),
    }
}

/// Séquenceur. Il est réutilisable : chaque appel à [`Sequencer::run`] donne les mêmes lectures
/// pour le même pool.
pub struct Sequencer {
    config: SequencerConfig,
}

impl Sequencer {
    /// Crée un séquenceur ; la configuration est vérifiée.
    pub fn new(config: SequencerConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Configuration du séquenceur.
    pub fn config(&self) -> &SequencerConfig {
        &self.config
    }

    /// Longueur des lectures d'index pour cette bibliothèque.
    pub fn index_read_length(&self, library: &Library) -> usize {
        self.config
            .index_read_length
            .unwrap_or_else(|| library.sample(0).i7.bases.len())
    }

    /// Séquence le pool et rend les paires de lectures, dans un ordre aléatoire.
    ///
    /// Chaque molécule tire de ses propres flux, dérivés de son rang : le résultat pour une
    /// molécule ne dépend ni de l'ordre ni du nombre des autres.
    pub fn run(
        &self,
        library: &Library,
        molecules: &[PoolMolecule],
    ) -> Result<(Vec<ReadPair>, SequencerStats)> {
        let config = &self.config;
        let index_length = self.index_read_length(library);
        let with_i5 = library.sample(0).i5.is_some();
        let templates: Vec<IndexTemplates> = molecules
            .iter()
            .map(|molecule| index_templates(molecule, index_length, with_i5))
            .collect();
        let mut stats = SequencerStats {
            molecules: molecules.len(),
            copies_min: u32::MAX,
            ..SequencerStats::default()
        };
        let mut pairs: Vec<ReadPair> = Vec::new();
        for (index, molecule) in molecules.iter().enumerate() {
            let key = index as u64 ^ config.seed;
            let (before, after) = library.primer_offsets(molecule.sample)?;
            let insert_end = molecule.sequence.len() - after;
            let insert = &molecule.sequence[before..insert_end];
            let dropped = config.dropout > 0.0
                && SeededRng::new(key, STREAM_DROPOUT).next_f64() < config.dropout;
            let (template, synthesis_errors) = if config.synthesis.total() > 0.0 {
                apply_errors(
                    insert,
                    &config.synthesis,
                    &mut SeededRng::new(key, STREAM_SYNTHESIS),
                )
            } else {
                (insert.to_vec(), ErrorCounts::default())
            };
            stats.synthesis_errors += synthesis_errors;
            let copies = if dropped {
                0
            } else {
                poisson(config.coverage, &mut SeededRng::new(key, STREAM_POISSON))
            };
            if copies == 0 {
                stats.lost_molecules += 1;
                continue;
            }
            stats.copies_min = stats.copies_min.min(copies);
            stats.copies_max = stats.copies_max.max(copies);

            let mut rng = SeededRng::new(key, STREAM_SEQUENCING);
            let mut rng_index = SeededRng::new(key, STREAM_INDEX);
            let mut rng_hopping = SeededRng::new(key, STREAM_HOPPING);
            for _ in 0..copies {
                // La lecture 1 part du début de l'insert, le traverse, puis lit l'adaptateur
                // qui le suit, jusqu'au bout de la molécule.
                let mut r1 = template[..config.read_length.min(template.len())].to_vec();
                let through = config.read_length.saturating_sub(template.len()).min(after);
                r1.extend_from_slice(&molecule.sequence[insert_end..insert_end + through]);
                // La lecture 2 part de l'extrémité mosaïque qui suit l'insert, lit l'insert à
                // l'envers, puis ce qui le précède, et s'arrête au bout de la molécule.
                let mut r2 = MOSAIC_END.to_vec();
                let in_insert = config
                    .read_length
                    .saturating_sub(MOSAIC_END.len())
                    .min(template.len());
                r2.extend_from_slice(&reverse_complement(&template[template.len() - in_insert..]));
                let before_insert = config
                    .read_length
                    .saturating_sub(MOSAIC_END.len() + template.len())
                    .min(before);
                if before_insert > 0 {
                    r2.extend_from_slice(&reverse_complement(
                        &molecule.sequence[before - before_insert..before],
                    ));
                }
                // Une lecture plus courte que l'extrémité mosaïque s'arrête dans celle-ci.
                r2.truncate(config.read_length);
                let (r1, quality_r1, errors_r1) =
                    sequence_read(&r1, &config.sequencing, true, &config.bursts, &mut rng);
                let (r2, quality_r2, errors_r2) =
                    sequence_read(&r2, &config.sequencing, true, &config.bursts, &mut rng);
                let errors = errors_r1 + errors_r2;
                stats.sequencing_errors += errors;

                // Les index sont ceux de la molécule ou, par un saut, ceux d'une autre.
                let donor = (config.index_hopping > 0.0
                    && rng_hopping.next_f64() < config.index_hopping)
                    .then(|| rng_hopping.below(molecules.len() as u64) as usize)
                    .filter(|&donor| donor != index);
                let source = &templates[donor.unwrap_or(index)];
                let (i7, quality_i7, errors_i7) = sequence_read(
                    &source.i7,
                    &config.index_sequencing,
                    false,
                    &BurstConfig::NONE,
                    &mut rng_index,
                );
                let (i5, quality_i5, errors_i5) = match &source.i5 {
                    Some(template) => {
                        let (bases, quality, errors) = sequence_read(
                            template,
                            &config.index_sequencing,
                            false,
                            &BurstConfig::NONE,
                            &mut rng_index,
                        );
                        (Some(bases), Some(quality), errors)
                    }
                    None => (None, None, ErrorCounts::default()),
                };
                let index_errors = errors_i7 + errors_i5;
                stats.index_errors += index_errors;
                stats.hopped += usize::from(donor.is_some());
                stats.pairs_perfect += usize::from(errors.total() == 0);
                pairs.push(ReadPair {
                    molecule: index as u32,
                    r1,
                    quality_r1,
                    r2,
                    quality_r2,
                    i7,
                    quality_i7,
                    i5,
                    quality_i5,
                    indexes_from: donor.map(|donor| donor as u32),
                    errors,
                    index_errors,
                });
            }
        }
        if stats.copies_min == u32::MAX {
            stats.copies_min = 0;
        }
        stats.pairs = pairs.len();

        let mut rng = SeededRng::new(config.seed, STREAM_SHUFFLE);
        for i in (1..pairs.len()).rev() {
            pairs.swap(i, rng.below(i as u64 + 1) as usize);
        }
        Ok((pairs, stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adn_core::library::{Index, Sample};
    use std::collections::{HashMap, HashSet};

    fn library() -> Library {
        Library::new(vec![
            Sample {
                name: "alpha".to_string(),
                i5: Some(Index::nextera_i5("N501").unwrap()),
                i7: Index::nextera_i7("N701").unwrap(),
            },
            Sample {
                name: "bêta".to_string(),
                i5: Some(Index::nextera_i5("N502").unwrap()),
                i7: Index::nextera_i7("N702").unwrap(),
            },
        ])
        .unwrap()
    }

    /// Un pool de `count` molécules d'inserts aléatoires de `length` bases, à parts égales
    /// entre les deux échantillons.
    fn pool(count: usize, length: usize) -> (Library, Vec<PoolMolecule>) {
        let library = library();
        let mut rng = SeededRng::new(9, 9);
        let molecules = (0..count)
            .map(|rank| {
                let mut bases = vec![0u8; length];
                rng.fill_bytes(&mut bases);
                bases
                    .iter_mut()
                    .for_each(|base| *base = BASES[(*base & 3) as usize]);
                PoolMolecule {
                    sample: rank % 2,
                    sequence: library.molecule(rank % 2, &bases).unwrap(),
                }
            })
            .collect();
        (library, molecules)
    }

    fn perfect() -> SequencerConfig {
        SequencerConfig {
            sequencing: ErrorRates::NONE,
            index_sequencing: ErrorRates::NONE,
            coverage: 20.0,
            ..SequencerConfig::default()
        }
    }

    /// L'insert d'une molécule, d'après les repères de son échantillon.
    fn insert<'a>(library: &Library, molecule: &'a PoolMolecule) -> &'a [u8] {
        let (before, after) = library.primer_offsets(molecule.sample).unwrap();
        &molecule.sequence[before..molecule.sequence.len() - after]
    }

    #[test]
    fn config_is_validated() {
        assert!(Sequencer::new(SequencerConfig::default()).is_ok());
        for config in [
            SequencerConfig {
                read_length: 0,
                ..SequencerConfig::default()
            },
            SequencerConfig {
                index_read_length: Some(0),
                ..SequencerConfig::default()
            },
            SequencerConfig {
                sequencing: ErrorRates::new(0.6, 0.0, 0.0),
                ..SequencerConfig::default()
            },
            SequencerConfig {
                index_hopping: 1.5,
                ..SequencerConfig::default()
            },
            SequencerConfig {
                coverage: -1.0,
                ..SequencerConfig::default()
            },
            SequencerConfig {
                dropout: f64::NAN,
                ..SequencerConfig::default()
            },
        ] {
            assert!(Sequencer::new(config).is_err(), "{config:?}");
        }
        // Cinquante pour cent d'erreur en moyenne donnent quatre-vingt-onze en fin de lecture :
        // ça passe encore.
        assert!(Sequencer::new(SequencerConfig {
            sequencing: ErrorRates::new(0.5, 0.0, 0.0),
            ..SequencerConfig::default()
        })
        .is_ok());
    }

    #[test]
    fn without_errors_reads_rebuild_the_inserts() {
        let (library, molecules) = pool(60, 152);
        let sequencer = Sequencer::new(perfect()).unwrap();
        let (pairs, stats) = sequencer.run(&library, &molecules).unwrap();
        assert_eq!(stats.pairs, pairs.len());
        // Soixante molécules, vingt lectures en moyenne : environ 1 200 paires.
        assert!((900..=1500).contains(&stats.pairs), "{}", stats.pairs);
        assert_eq!(stats.lost_molecules, 0);
        assert_eq!(stats.pairs_perfect, stats.pairs);
        assert_eq!(stats.hopped, 0);
        assert!(stats.copies_min >= 5, "{}", stats.copies_min);
        assert_eq!(sequencer.index_read_length(&library), 8);
        for pair in &pairs {
            let molecule = &molecules[pair.molecule as usize];
            let sample = library.sample(molecule.sample);
            let (before, _) = library.primer_offsets(molecule.sample).unwrap();
            let sequence = &molecule.sequence;
            // Lecture 1 : l'insert, puis ce qui le suit. La molécule s'arrête avant trois cents
            // bases.
            assert_eq!(pair.r1.len(), 152 + 66);
            assert_eq!(pair.r1, sequence[before..]);
            // Lecture 2 : l'extrémité mosaïque, l'insert à l'envers, puis ce qui le précède,
            // jusqu'au bout de la molécule.
            assert_eq!(pair.r2.len(), 19 + 152 + before);
            assert!(pair.r2.starts_with(MOSAIC_END));
            assert_eq!(
                &pair.r2[19..19 + 152],
                reverse_complement(insert(&library, molecule)).as_slice()
            );
            // Index : i7 complément inversé, i5 dans le sens de la molécule.
            assert_eq!(pair.i7, reverse_complement(&sample.i7.bases));
            assert!(pair.i7 == b"TCGCCTTA" || pair.i7 == b"CTAGTACG");
            assert_eq!(
                pair.i5.as_deref(),
                Some(sample.i5.as_ref().unwrap().bases.as_slice())
            );
            // Qualité maximale partout, aucune erreur.
            for quality in [
                &pair.quality_r1,
                &pair.quality_r2,
                &pair.quality_i7,
                pair.quality_i5.as_ref().unwrap(),
            ] {
                assert!(quality.iter().all(|&q| q == MAX_PHRED));
            }
            assert_eq!(pair.errors.total(), 0);
            assert_eq!(pair.index_errors.total(), 0);
            assert!(pair.indexes_from.is_none());
        }
    }

    #[test]
    fn reads_stop_at_the_ends_of_the_molecule() {
        let (library, molecules) = pool(4, 117);
        let sequencer = Sequencer::new(SequencerConfig {
            read_length: 10_000,
            ..perfect()
        })
        .unwrap();
        let (pairs, _) = sequencer.run(&library, &molecules).unwrap();
        assert!(!pairs.is_empty());
        for pair in &pairs {
            let molecule = &molecules[pair.molecule as usize];
            let (before, after) = library.primer_offsets(molecule.sample).unwrap();
            assert_eq!(pair.r1.len(), 117 + after);
            assert_eq!(pair.r2.len(), 19 + 117 + before);
        }
        // Lecture plus courte que l'extrémité mosaïque : rien de l'insert.
        let (short, _) = Sequencer::new(SequencerConfig {
            read_length: 10,
            ..perfect()
        })
        .unwrap()
        .run(&library, &molecules)
        .unwrap();
        assert!(!short.is_empty());
        for pair in &short {
            assert_eq!(pair.r1.len(), 10);
            assert_eq!(pair.r2, MOSAIC_END[..10]);
            assert_eq!(pair.i7.len(), 8);
        }
    }

    #[test]
    fn errors_grow_along_the_read_and_match_the_quality() {
        let (library, molecules) = pool(40, 2000);
        let config = SequencerConfig {
            read_length: 2000,
            sequencing: ErrorRates::new(0.05, 0.0, 0.0),
            coverage: 1.0,
            ..SequencerConfig::default()
        };
        let (pairs, stats) = Sequencer::new(config)
            .unwrap()
            .run(&library, &molecules)
            .unwrap();
        assert!(stats.pairs >= 25, "{}", stats.pairs);
        let mut mismatches = [0usize; 4];
        for pair in &pairs {
            let original = insert(&library, &molecules[pair.molecule as usize]);
            assert_eq!(pair.r1.len(), 2000);
            // Qualité décroissante le long de la lecture.
            assert!(pair.quality_r1.windows(2).all(|w| w[0] >= w[1]));
            for (quarter, counted) in mismatches.iter_mut().enumerate() {
                let range = quarter * 500..(quarter + 1) * 500;
                *counted += pair.r1[range.clone()]
                    .iter()
                    .zip(&original[range])
                    .filter(|(&a, &b)| a != b)
                    .count();
            }
        }
        let rate = |quarter: usize| mismatches[quarter] as f64 / (500.0 * pairs.len() as f64);
        for quarter in 0..4 {
            // Moyenne de la rampe sur le quart : (bornes + 0,2) / 1,2.
            let expected = 0.05 * (quarter as f64 / 2.0 + 0.45) / 1.2;
            assert!(
                (rate(quarter) - expected).abs() < 0.006,
                "quart {quarter} : {} pour {} attendu",
                rate(quarter),
                expected
            );
        }
        assert!(rate(3) > 4.0 * rate(0));
        // La qualité inscrite est celle de la probabilité d'erreur en début et en fin de lecture.
        assert_eq!(pairs[0].quality_r1[0], 21);
        assert_eq!(pairs[0].quality_r1[1999], 10);
        assert_eq!(pairs[0].quality_i7[0], 30);
    }

    #[test]
    fn same_seed_gives_same_pairs_and_pairs_are_shuffled() {
        let (library, molecules) = pool(40, 152);
        let sequencer = Sequencer::new(SequencerConfig {
            coverage: 4.0,
            ..SequencerConfig::default()
        })
        .unwrap();
        let (first, stats_first) = sequencer.run(&library, &molecules).unwrap();
        let (second, stats_second) = sequencer.run(&library, &molecules).unwrap();
        assert_eq!(first, second);
        assert_eq!(stats_first, stats_second);
        // Environ 160 paires de 300 bases à 0,1 % : une soixantaine de substitutions.
        assert!(stats_first.sequencing_errors.substitutions > 20);
        assert!(stats_first.index_errors.substitutions > 0);
        assert!(first.windows(2).any(|w| w[0].molecule > w[1].molecule));
        // Une autre graine, d'autres lectures.
        let (other, _) = Sequencer::new(SequencerConfig {
            coverage: 4.0,
            seed: 43,
            ..SequencerConfig::default()
        })
        .unwrap()
        .run(&library, &molecules)
        .unwrap();
        assert_ne!(first, other);
    }

    #[test]
    fn index_hopping_borrows_the_indexes_of_another_molecule() {
        let (library, molecules) = pool(60, 117);
        let (clean, _) = Sequencer::new(perfect())
            .unwrap()
            .run(&library, &molecules)
            .unwrap();
        for pair in &clean {
            assert!(pair.indexes_from.is_none());
            let sample = library.sample(molecules[pair.molecule as usize].sample);
            assert_eq!(pair.i7, reverse_complement(&sample.i7.bases));
            assert_eq!(
                pair.i5.as_deref(),
                Some(sample.i5.as_ref().unwrap().bases.as_slice())
            );
        }
        // Tout saute : les index sont ceux d'une autre molécule, intacts, et le compte en est
        // fait. Une paire sur soixante tombe sur sa propre molécule : elle ne compte pas.
        let (pairs, stats) = Sequencer::new(SequencerConfig {
            index_hopping: 1.0,
            ..perfect()
        })
        .unwrap()
        .run(&library, &molecules)
        .unwrap();
        let hopped = pairs
            .iter()
            .filter(|pair| pair.indexes_from.is_some())
            .count();
        assert!(
            hopped >= pairs.len() * 9 / 10,
            "{hopped} sur {}",
            pairs.len()
        );
        assert_eq!(stats.hopped, hopped);
        for pair in pairs.iter().filter(|pair| pair.indexes_from.is_some()) {
            let donor = library.sample(molecules[pair.indexes_from.unwrap() as usize].sample);
            assert_eq!(pair.i7, reverse_complement(&donor.i7.bases));
            assert_eq!(
                pair.i5.as_deref(),
                Some(donor.i5.as_ref().unwrap().bases.as_slice())
            );
            // Les lectures, elles, restent celles de la molécule d'origine.
            assert_eq!(
                pair.r1[..117],
                insert(&library, &molecules[pair.molecule as usize])[..]
            );
        }
    }

    #[test]
    fn synthesis_errors_are_shared_by_the_pairs_of_a_molecule() {
        let (library, molecules) = pool(80, 117);
        let config = SequencerConfig {
            synthesis: ErrorRates::new(0.05, 0.0, 0.0),
            read_length: 19 + 117,
            ..perfect()
        };
        let (pairs, stats) = Sequencer::new(config)
            .unwrap()
            .run(&library, &molecules)
            .unwrap();
        assert!(stats.synthesis_errors.substitutions > 300);
        assert_eq!(stats.pairs_perfect, stats.pairs);
        let mut first_read: HashMap<u32, [Vec<u8>; 2]> = HashMap::new();
        let mut damaged = HashSet::new();
        for pair in &pairs {
            let molecule = pair.molecule;
            let original = insert(&library, &molecules[molecule as usize]);
            let reads = [pair.r1.clone(), pair.r2[19..19 + 117].to_vec()];
            if pair.r1[..] != original[..117] {
                damaged.insert(molecule);
            }
            match first_read.get(&molecule) {
                None => {
                    first_read.insert(molecule, reads);
                }
                Some(seen) => assert_eq!(
                    seen, &reads,
                    "les lectures de la molécule {molecule} portent les mêmes erreurs de \
                     synthèse"
                ),
            }
        }
        // Une erreur de synthèse par vingt bases : presque chaque molécule en porte.
        assert!(damaged.len() >= first_read.len() * 9 / 10);
    }

    #[test]
    fn molecules_without_any_read_are_lost() {
        let (library, molecules) = pool(200, 117);
        let (pairs, stats) = Sequencer::new(SequencerConfig {
            coverage: 0.01,
            ..perfect()
        })
        .unwrap()
        .run(&library, &molecules)
        .unwrap();
        // Une couverture de 0,01 laisse une molécule sur cent sans lecture : presque toutes.
        assert!(stats.lost_molecules >= 190, "{}", stats.lost_molecules);
        let read: HashSet<u32> = pairs.iter().map(|pair| pair.molecule).collect();
        assert_eq!(stats.lost_molecules, 200 - read.len());
        // Pool entièrement perdu.
        let (pairs, stats) = Sequencer::new(SequencerConfig {
            dropout: 1.0,
            ..perfect()
        })
        .unwrap()
        .run(&library, &molecules)
        .unwrap();
        assert!(pairs.is_empty());
        assert_eq!(stats.lost_molecules, 200);
        assert_eq!(stats.copies_min, 0);
    }

    #[test]
    fn bursts_strike_the_reads_and_spare_the_indexes() {
        let (library, molecules) = pool(40, 117);
        let config = SequencerConfig {
            sequencing: ErrorRates::NONE,
            index_sequencing: ErrorRates::NONE,
            bursts: BurstConfig {
                rate: 0.02,
                length: 20.0,
                error: 1.0,
            },
            coverage: 2.0,
            ..SequencerConfig::default()
        };
        assert!(Sequencer::new(config).is_ok());
        let (pairs, stats) = Sequencer::new(config)
            .unwrap()
            .run(&library, &molecules)
            .unwrap();
        assert_eq!(pairs.len(), stats.pairs);
        // Environ 29 % des bases des lectures sont fausses ; les index, aucune.
        let wrong: usize = pairs
            .iter()
            .map(|pair| {
                let molecule = &molecules[pair.molecule as usize];
                let (before, _) = library.primer_offsets(molecule.sample).unwrap();
                let truth = &molecule.sequence[before..before + pair.r1.len()];
                truth.iter().zip(&pair.r1).filter(|(a, b)| a != b).count()
            })
            .sum();
        let bases: usize = pairs.iter().map(|pair| pair.r1.len()).sum();
        let rate = wrong as f64 / bases as f64;
        assert!((0.25..=0.33).contains(&rate), "{rate}");
        for pair in &pairs {
            let sample = library.sample(molecules[pair.molecule as usize].sample);
            assert_eq!(
                pair.i7,
                reverse_complement(&sample.i7.bases),
                "les lectures d'index ne prennent pas de salve"
            );
            assert!(pair.quality_r1.iter().all(|&score| score == MAX_PHRED));
        }
        // Une configuration incohérente est refusée.
        let bad = SequencerConfig {
            bursts: BurstConfig {
                rate: 3.0,
                ..BurstConfig::NONE
            },
            ..SequencerConfig::default()
        };
        assert!(Sequencer::new(bad).is_err());
    }

    #[test]
    fn headers_carry_the_index_reads_and_the_origin() {
        let (library, molecules) = pool(2, 117);
        let (pairs, _) = Sequencer::new(perfect())
            .unwrap()
            .run(&library, &molecules)
            .unwrap();
        let pair = &pairs[0];
        let text = |bases: &[u8]| bases.iter().map(|&b| b as char).collect::<String>();
        let header = read_header(&read_id(0), 1, pair, Some("alpha"));
        assert_eq!(
            header,
            format!(
                "SIM:1:FC:1:1101:0:0 1:N:0:{}+{} sample=alpha",
                text(&pair.i7),
                text(pair.i5.as_ref().unwrap())
            )
        );
        assert_eq!(truth_sample(&header), Some("alpha"));
        assert_eq!(truth_sample(&read_header("x", 3, pair, None)), None);
        assert_eq!(read_id(1001), "SIM:1:FC:1:1101:1:1");
    }
}
