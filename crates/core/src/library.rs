//! Bibliothèques de séquençage Illumina : adaptateurs, index, multiplexage.
//!
//! Une archive est un jeu d'inserts. Pour les faire lire par un séquenceur, chaque insert devient
//! une molécule, ici au format Nextera (Illumina, *Illumina Adapter Sequences*, document
//! 1000000002694) :
//!
//! | P5 | index i5 | TCGTCGGCAGCGTC | ME | insert | MEᶜ | CCGAGCCCACGAGAC | index i7 | P7ᶜ |
//!
//! - **P5** : `AATGATACGGCGACCACCGAGATCTACAC`. **P7** : `CAAGCAGAAGACGGCATACGAGAT`, qui
//!   apparaît complément inversé en fin de molécule.
//! - **ME**, l'extrémité mosaïque de la transposase : `AGATGTGTATAAGAGACAG`. Elle revient,
//!   complément inversée, après l'insert.
//! - La **lecture 1** s'amorce au site `TCGTCGGCAGCGTC` : elle lit l'insert, puis ce qui le
//!   suit.
//! - La **lecture 2** s'amorce au site `CCGAGCCCACGAGAC` : elle commence par ME, lit l'insert à
//!   l'envers, puis ce qui le précède.
//! - La **lecture d'index i7** s'amorce avec P7, celle d'**i5** avec P5. Les index identifient
//!   l'échantillon : plusieurs archives peuvent être mélangées dans une même cellule, puis
//!   séparées à la lecture. C'est le multiplexage.
//!
//! Le démultiplexage tolère une erreur par index. Pour que l'erreur ne puisse pas faire tomber
//! une lecture sur un autre échantillon, deux index d'une même position doivent différer d'au
//! moins trois bases, et la bibliothèque le vérifie. Le sens d'un index lu dépend de la
//! plateforme : il est donc essayé dans les deux sens.
//!
//! Les index du kit Nextera sont donnés dans le sens de la molécule : pour i7, les bases de la
//! feuille d'échantillon du document d'Illumina ; pour i5, celles de l'adaptateur.
//!
//! [`Library::molecule`] entoure un insert de ses adaptateurs, [`place`] range une paire de
//! lectures dans son échantillon et coupe les adaptateurs — une lecture 2 arrêtée dans l'insert
//! est écartée —, [`read_pool`] relit un pool écrit par la commande `adn library`.
//!
//! ```
//! use adn_core::library::{place, Library, Placement};
//! use adn_core::sequence::reverse_complement;
//! use adn_core::{Index, Sample};
//!
//! let library = Library::new(vec![
//!     Sample {
//!         name: "photo".into(),
//!         i7: Index::nextera_i7("N701").unwrap(),
//!         i5: Some(Index::nextera_i5("N501").unwrap()),
//!     },
//!     Sample {
//!         name: "chanson".into(),
//!         i7: Index::nextera_i7("N702").unwrap(),
//!         i5: Some(Index::nextera_i5("N502").unwrap()),
//!     },
//! ])
//! .unwrap();
//!
//! // Un oligo d'archive devient l'insert d'une molécule : 136 nt d'adaptateurs l'entourent.
//! let insert = b"ACGTTGCAACGTTGCAACGTTGCA";
//! let molecule = library.molecule(0, insert).unwrap();
//! assert_eq!(molecule.len(), insert.len() + 136);
//!
//! // Les lectures telles qu'un séquenceur les tire, jusqu'au bout de la molécule.
//! let (before, _) = library.primer_offsets(0).unwrap();
//! let r1 = &molecule[before..];
//! let r2 = reverse_complement(&molecule[..before + insert.len() + 19]);
//! let sample = library.sample(0);
//! let mut i7 = reverse_complement(&sample.i7.bases);
//! i7[0] = if i7[0] == b'A' { b'C' } else { b'A' };
//! // L'autre index, lu dans l'autre sens selon la plateforme.
//! let i5 = reverse_complement(&sample.i5.as_ref().unwrap().bases);
//!
//! // Une erreur dans un index, un sens de lecture par index : la paire est rangée dans son
//! // échantillon, ses lectures rendues dans le sens de l'insert, coupées des adaptateurs.
//! match place(&library, r1, &r2, &i7, Some(&i5)) {
//!     Placement::Sample(reads) => {
//!         assert_eq!(reads.sample, 0);
//!         assert_eq!(reads.r1, insert);
//!         assert_eq!(reads.r2, insert);
//!     }
//!     other => panic!("{other:?}"),
//! }
//! ```

use crate::error::{DnaError, Result};
use crate::sequence::{reverse_complement, BASES};

/// Adaptateur P5, en tête de molécule.
pub const P5: &[u8] = b"AATGATACGGCGACCACCGAGATCTACAC";
/// Adaptateur P7, tel qu'il figure dans le document d'Illumina.
pub const P7: &[u8] = b"CAAGCAGAAGACGGCATACGAGAT";
/// Site d'amorce de la lecture 1, entre l'index i5 et l'extrémité mosaïque.
pub const READ1_PRIMER_SITE: &[u8] = b"TCGTCGGCAGCGTC";
/// Site d'amorce de la lecture 2, tel qu'il apparaît dans la molécule.
pub const READ2_PRIMER_SITE: &[u8] = b"CCGAGCCCACGAGAC";
/// Extrémité mosaïque de la transposase, avant l'insert.
pub const MOSAIC_END: &[u8] = b"AGATGTGTATAAGAGACAG";
/// Extrémité mosaïque après l'insert, dans la molécule : ce que lit une lecture qui traverse
/// l'insert.
pub const READ_THROUGH: &[u8] = b"CTGTCTCTTATACACATCT";

/// Deux index d'une même position diffèrent d'au moins ce nombre de bases.
pub const MIN_INDEX_DISTANCE: usize = 3;
/// Longueur minimale d'un index.
pub const MIN_INDEX_LEN: usize = 4;
/// Longueur maximale d'un index.
pub const MAX_INDEX_LEN: usize = 12;
/// Longueur minimale d'un dépassement d'insert que le découpage reconnaît.
pub const MIN_READ_THROUGH: usize = 10;

/// Index i7 du kit Nextera XT v2, dans le sens de la molécule.
pub const NEXTERA_I7: [(&str, &str); 12] = [
    ("N701", "TAAGGCGA"),
    ("N702", "CGTACTAG"),
    ("N703", "AGGCAGAA"),
    ("N704", "TCCTGAGC"),
    ("N705", "GGACTCCT"),
    ("N706", "TAGGCATG"),
    ("N707", "CTCTCTAC"),
    ("N708", "CAGAGAGG"),
    ("N709", "GCTACGCT"),
    ("N710", "CGAGGCTG"),
    ("N711", "AAGAGGCA"),
    ("N712", "GTAGAGGA"),
];

/// Index i5 du kit Nextera (N501 à N508), dans le sens de la molécule.
pub const NEXTERA_I5: [(&str, &str); 8] = [
    ("N501", "TAGATCGC"),
    ("N502", "CTCTCTAT"),
    ("N503", "TATCCTCT"),
    ("N504", "AGAGTAGA"),
    ("N505", "GTAAGGAG"),
    ("N506", "ACTGCATA"),
    ("N507", "AAGGAGTA"),
    ("N508", "CTAAGCCT"),
];

/// Un index de séquençage : un nom et sa séquence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub bases: Vec<u8>,
}

impl Index {
    /// Un index de séquence donnée, vérifiée.
    pub fn new(name: impl Into<String>, bases: &str) -> Result<Self> {
        let name = name.into();
        // Le nom d'un index se retrouve dans un en-tête FASTA, à la suite d'un signe =, et ses
        // bases à la suite d'un signe :.
        if name.is_empty()
            || name
                .chars()
                .any(|c| c.is_whitespace() || c == '=' || c == ':')
        {
            return Err(DnaError::InvalidConfig(format!(
                "index « {name} » : nom invalide (ni vide, ni blanc, ni signe = ou :)"
            )));
        }
        let bases = bases.trim().to_ascii_uppercase();
        if !(MIN_INDEX_LEN..=MAX_INDEX_LEN).contains(&bases.len()) {
            return Err(DnaError::InvalidConfig(format!(
                "index {name} : {} bases (attendu : {} à {})",
                bases.len(),
                MIN_INDEX_LEN,
                MAX_INDEX_LEN
            )));
        }
        if !bases.bytes().all(|base| BASES.contains(&base)) {
            return Err(DnaError::InvalidConfig(format!(
                "index {name} : {bases} ne contient pas que A, C, G, T"
            )));
        }
        Ok(Self {
            name,
            bases: bases.into_bytes(),
        })
    }

    /// Un index i7 du kit Nextera XT v2 (`N701` à `N712`), par son nom.
    pub fn nextera_i7(name: &str) -> Option<Self> {
        Self::from_table(&NEXTERA_I7, name)
    }

    /// Un index i5 du kit Nextera (`N501` à `N508`), par son nom.
    pub fn nextera_i5(name: &str) -> Option<Self> {
        Self::from_table(&NEXTERA_I5, name)
    }

    /// Un index d'un kit Nextera, par son nom (`N501` à `N508`, `N701` à `N712`).
    pub fn nextera(name: &str) -> Option<Self> {
        Self::nextera_i7(name).or_else(|| Self::nextera_i5(name))
    }

    fn from_table(table: &[(&str, &str)], name: &str) -> Option<Self> {
        let key = name.trim().to_ascii_uppercase();
        table
            .iter()
            .find(|(known, _)| *known == key)
            .and_then(|(known, bases)| Self::new(*known, bases).ok())
    }

    /// Nom et séquence, pour un en-tête FASTA.
    pub fn describe(&self) -> String {
        format!("{}={}", self.name, String::from_utf8_lossy(&self.bases))
    }
}

/// Un échantillon d'une bibliothèque : un nom, son index i7 et, pour un multiplexage à deux
/// index, son index i5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub name: String,
    pub i5: Option<Index>,
    pub i7: Index,
}

/// Une bibliothèque : des échantillons aux index assez distants pour être départagés malgré une
/// erreur par index. Les échantillons y sont rangés par nom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    samples: Vec<Sample>,
}

/// À quel échantillon appartient une lecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assignment {
    /// L'échantillon de ce rang, avec ce nombre d'erreurs dans ses index.
    Sample { sample: usize, mismatches: usize },
    /// Deux échantillons aussi proches : on ne tranche pas.
    Ambiguous,
    /// Aucun échantillon à une erreur ou moins.
    Unknown,
}

fn invalid(what: impl std::fmt::Display) -> DnaError {
    DnaError::InvalidConfig(format!("bibliothèque invalide : {what}"))
}

/// Nombre de bases différentes entre deux séquences, longueur comprise.
fn differences(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(&x, &y)| x != y).count() + a.len().abs_diff(b.len())
}

/// Nom et séquence d'un index, pour un en-tête de pool.
fn described(index: &Index) -> String {
    format!("{}:{}", index.name, String::from_utf8_lossy(&index.bases))
}

impl Library {
    /// Vérifie les échantillons et leurs index.
    ///
    /// Deux index d'une même position diffèrent d'au moins trois bases : une erreur ne les
    /// confond pas. Et aucun n'est le complément inversé d'un autre : les index étant comparés
    /// dans les deux sens de lecture, une lecture exacte resterait sinon entre deux échantillons.
    /// Les noms sont uniques ; les index d'une même position ont même longueur ; l'index i5 est
    /// présent partout ou nulle part.
    pub fn new(mut samples: Vec<Sample>) -> Result<Self> {
        if samples.is_empty() {
            return Err(invalid("aucun échantillon"));
        }
        samples.sort_by(|a, b| a.name.cmp(&b.name));
        // Le nom d'un échantillon se retrouve dans un en-tête FASTA, à la suite d'un signe =
        // et jusqu'au premier blanc.
        let name_is_valid = |name: &str| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| !c.is_whitespace() && c != '=' && c != ':')
        };
        if let Some(sample) = samples.iter().find(|sample| !name_is_valid(&sample.name)) {
            return Err(invalid(format!(
                "nom d'échantillon invalide : « {} » (ni vide, ni blanc, ni signe = ou :)",
                sample.name
            )));
        }
        for pair in samples.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(invalid("deux échantillons portent le même nom"));
            }
        }
        let with_i5 = samples.iter().filter(|sample| sample.i5.is_some()).count();
        if with_i5 != 0 && with_i5 != samples.len() {
            return Err(invalid("tous les échantillons n'ont pas d'index i5"));
        }
        for (position, index_of) in [
            (
                "i5",
                (|sample: &Sample| sample.i5.as_ref()) as fn(&Sample) -> Option<&Index>,
            ),
            (
                "i7",
                (|sample: &Sample| Some(&sample.i7)) as fn(&Sample) -> Option<&Index>,
            ),
        ] {
            for (rank, sample) in samples.iter().enumerate() {
                let Some(index) = index_of(sample) else {
                    continue;
                };
                for other in &samples[..rank] {
                    let Some(theirs) = index_of(other) else {
                        continue;
                    };
                    if index.bases.len() != theirs.bases.len() {
                        return Err(invalid(format!(
                            "les index {position} n'ont pas tous la même longueur"
                        )));
                    }
                    let flipped = differences(&reverse_complement(&index.bases), &theirs.bases);
                    if flipped == 0 {
                        return Err(invalid(format!(
                            "index {position} : {} est le complément inversé de {} ; selon le \
                             sens de lecture, une lecture exacte les confondrait",
                            index.name, theirs.name
                        )));
                    }
                    let distance = differences(&index.bases, &theirs.bases);
                    if distance < MIN_INDEX_DISTANCE {
                        return Err(invalid(format!(
                            "index {position} trop proches : {} et {} ({distance} base(s) de \
                             différence, {MIN_INDEX_DISTANCE} nécessaires)",
                            index.name, theirs.name
                        )));
                    }
                }
            }
        }
        Ok(Self { samples })
    }

    /// Les échantillons, rangés par nom.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// L'échantillon de rang `sample`.
    pub fn sample(&self, sample: usize) -> &Sample {
        &self.samples[sample]
    }

    /// Rang de l'échantillon de ce nom.
    pub fn rank(&self, name: &str) -> Option<usize> {
        self.samples.iter().position(|sample| sample.name == name)
    }

    /// La molécule d'un insert : les adaptateurs et les index de l'échantillon autour de lui.
    pub fn molecule(&self, sample: usize, insert: &[u8]) -> Result<Vec<u8>> {
        let sample = self
            .samples
            .get(sample)
            .ok_or_else(|| invalid("échantillon inconnu"))?;
        let Some(i5) = &sample.i5 else {
            return Err(invalid(format!(
                "{} n'a pas d'index i5 : un insert se place dans une molécule à deux index",
                sample.name
            )));
        };
        if insert.is_empty() {
            return Err(invalid("insert vide"));
        }
        let mut molecule = Vec::with_capacity(
            P5.len()
                + i5.bases.len()
                + READ1_PRIMER_SITE.len()
                + MOSAIC_END.len()
                + insert.len()
                + READ_THROUGH.len()
                + READ2_PRIMER_SITE.len()
                + sample.i7.bases.len()
                + P7.len(),
        );
        molecule.extend_from_slice(P5);
        molecule.extend_from_slice(&i5.bases);
        molecule.extend_from_slice(READ1_PRIMER_SITE);
        molecule.extend_from_slice(MOSAIC_END);
        molecule.extend_from_slice(insert);
        molecule.extend_from_slice(READ_THROUGH);
        molecule.extend_from_slice(READ2_PRIMER_SITE);
        molecule.extend_from_slice(&sample.i7.bases);
        molecule.extend_from_slice(&reverse_complement(P7));
        Ok(molecule)
    }

    /// Début de l'insert dans la molécule de l'échantillon, et longueur de ce qui le suit.
    ///
    /// Ce sont les repères des lectures : la lecture 1 démarre au début de l'insert, la lecture
    /// 2 à sa fin, sur l'autre brin ; les lectures d'index se prennent aux deux bouts.
    pub fn primer_offsets(&self, sample: usize) -> Result<(usize, usize)> {
        let sample = self
            .samples
            .get(sample)
            .ok_or_else(|| invalid("échantillon inconnu"))?;
        let before = P5.len()
            + sample.i5.as_ref().map_or(0, |index| index.bases.len())
            + READ1_PRIMER_SITE.len()
            + MOSAIC_END.len();
        let after = P7.len() + sample.i7.bases.len() + READ2_PRIMER_SITE.len() + READ_THROUGH.len();
        Ok((before, after))
    }

    /// En-tête FASTA d'une molécule : son échantillon et ses index. [`read_pool`] le relit.
    pub fn molecule_header(&self, sample: usize) -> Result<String> {
        let sample = self
            .samples
            .get(sample)
            .ok_or_else(|| invalid("échantillon inconnu"))?;
        let i5 = sample
            .i5
            .as_ref()
            .map(|index| format!(" i5={}", described(index)))
            .unwrap_or_default();
        Ok(format!(
            "sample={} i7={}{i5}",
            sample.name,
            described(&sample.i7)
        ))
    }

    /// Rang de l'échantillon d'une paire de lectures d'index, ou la raison de ne pas trancher.
    ///
    /// Chaque index lu doit être à une erreur au plus d'un index d'échantillon : l'index i7
    /// toujours, l'index i5 quand il est donné et que la bibliothèque en a. Un seul échantillon
    /// peut satisfaire cette condition dès que les index sont à trois bases l'un de l'autre :
    /// deux non, et l'on ne tranche pas.
    pub fn demultiplex(&self, i7: &[u8], i5: Option<&[u8]>) -> Assignment {
        // Chaque index est comparé dans les deux sens de lecture.
        let distance = |read: &[u8], index: &Index| {
            differences(read, &index.bases)
                .min(differences(&reverse_complement(read), &index.bases))
        };
        let mut best: Option<(usize, usize)> = None;
        let mut tied = false;
        for (rank, sample) in self.samples.iter().enumerate() {
            let on_i7 = distance(i7, &sample.i7);
            let on_i5 = match (&sample.i5, i5) {
                (Some(index), Some(read)) => distance(read, index),
                _ => 0,
            };
            if on_i7 > 1 || on_i5 > 1 {
                continue;
            }
            let mismatches = on_i7 + on_i5;
            match best {
                Some((_, fewest)) if mismatches > fewest => {}
                Some((_, fewest)) if mismatches == fewest => tied = true,
                _ => {
                    best = Some((rank, mismatches));
                    tied = false;
                }
            }
        }
        match best {
            Some((sample, mismatches)) if !tied => Assignment::Sample { sample, mismatches },
            Some(_) => Assignment::Ambiguous,
            None => Assignment::Unknown,
        }
    }
}

/// Dépassement d'insert qui commence en `start`, s'il y en a un de reconnaissable.
fn read_through_at(read: &[u8], start: usize) -> bool {
    let Some(overlap) = read.len().checked_sub(start) else {
        return false;
    };
    let overlap = overlap.min(READ_THROUGH.len());
    if overlap < MIN_READ_THROUGH {
        return false;
    }
    let mismatches = differences(&read[start..start + overlap], &READ_THROUGH[..overlap]);
    // Un dépassement complet se reconnaît à une erreur près ; un début de dépassement, et il en
    // faut au moins dix, exactement.
    mismatches == 0 || (overlap == READ_THROUGH.len() && mismatches <= 1)
}

/// Fin d'une lecture réduite à la fin de l'extrémité mosaïque complément inversée : la longueur
/// de ce qui reste à couper. C'est le cas d'une lecture 2, complément inversée, qui s'est arrêtée
/// dans l'extrémité mosaïque par laquelle elle commence.
fn trailing_read_through(read: &[u8]) -> Option<usize> {
    let overlap = read.len().min(READ_THROUGH.len());
    if overlap < MIN_READ_THROUGH {
        return None;
    }
    let mismatches = differences(
        &read[read.len() - overlap..],
        &READ_THROUGH[READ_THROUGH.len() - overlap..],
    );
    (mismatches == 0 || (overlap == READ_THROUGH.len() && mismatches <= 1)).then_some(overlap)
}

/// Coupe, à la fin d'une lecture, ce qui dépasse l'insert.
///
/// La lecture 1 lit l'insert, puis rencontre l'extrémité mosaïque complément inversée
/// (`CTGTCTCTTATACACATCT`) : elle est coupée là. Il en va de même d'une lecture 2, complément
/// inversée, dont les dix-neuf dernières bases sont cette extrémité, ou qui s'est arrêtée dedans.
/// Un dépassement de moins de dix bases n'est pas reconnaissable : la lecture est rendue telle
/// quelle, et le décodeur en rejettera la longueur.
pub fn trim_after_insert(read: &[u8]) -> &[u8] {
    if let Some(overlap) = trailing_read_through(read) {
        return &read[..read.len() - overlap];
    }
    (0..read.len())
        .find(|&start| read_through_at(read, start))
        .map_or(read, |start| &read[..start])
}

/// Une paire de lectures rangée dans un échantillon : ses lectures, coupées des adaptateurs,
/// dans le sens de l'insert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertReads {
    pub sample: usize,
    pub r1: Vec<u8>,
    pub r2: Vec<u8>,
    /// La lecture 2 s'est arrêtée dans l'insert : elle n'en donnait que la fin, et `r2` est vide.
    pub r2_partial: bool,
}

/// Le résultat du rangement d'une paire de lectures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    Sample(InsertReads),
    Ambiguous,
    Unknown,
}

/// Range une paire de lectures dans son échantillon, d'après ses lectures d'index, et coupe les
/// adaptateurs.
///
/// Les lectures sont rendues dans le sens de l'insert : la lecture 1 depuis son début, la
/// lecture 2, complément inversée, depuis sa fin. Une lecture qui ne contient que des adaptateurs
/// y est vide. Une lecture 2 qui ne rejoint pas le début de l'insert n'en donne que la fin, qu'aucun
/// schéma ne lit : elle est vide aussi, et [`InsertReads::r2_partial`] le dit.
pub fn place(library: &Library, r1: &[u8], r2: &[u8], i7: &[u8], i5: Option<&[u8]>) -> Placement {
    match library.demultiplex(i7, i5) {
        Assignment::Sample { sample, .. } => {
            let r1 = trim_after_insert(r1);
            let flipped = reverse_complement(r2);
            let flipped = trim_after_insert(&flipped);
            let start = before_insert(flipped).unwrap_or(0);
            let r2 = &flipped[start..];
            // Sans extrémité mosaïque reconnaissable avant l'insert, une lecture 2 plus courte
            // que la lecture 1 s'est arrêtée dans l'insert : elle n'en donne que la fin. Aussi
            // longue ou plus longue, on ne peut rien conclure — et on la garde.
            let r2_partial = start == 0 && r2.len() < r1.len();
            Placement::Sample(InsertReads {
                sample,
                r1: r1.to_vec(),
                r2: if r2_partial { Vec::new() } else { r2.to_vec() },
                r2_partial,
            })
        }
        Assignment::Ambiguous => Placement::Ambiguous,
        Assignment::Unknown => Placement::Unknown,
    }
}

/// Ce qui précède l'insert dans une molécule, au plus : P5, l'index i5, le site d'amorce de la
/// lecture 1 et l'extrémité mosaïque.
const BEFORE_INSERT_MAX: usize =
    P5.len() + MAX_INDEX_LEN + READ1_PRIMER_SITE.len() + MOSAIC_END.len();

/// Coupe, au début d'une lecture, ce qui précède l'insert.
///
/// C'est le cas d'une lecture 2 qui a traversé tout l'insert, une fois complément inversée : ce
/// qu'elle a lu après l'insert se retrouve avant lui, et se termine par l'extrémité mosaïque
/// (`AGATGTGTATAAGAGACAG`), reconnue entière à une erreur près. Une lecture trop courte pour la
/// contenir n'en garde que la fin : si sa première base d'insert la prolonge, elle se confond
/// avec elle.
pub fn trim_before_insert(read: &[u8]) -> &[u8] {
    &read[before_insert(read).unwrap_or(0)..]
}

/// Ce qui précède l'insert au début d'une lecture, s'il est reconnaissable : le nombre de bases à
/// couper ([`trim_before_insert`]).
fn before_insert(read: &[u8]) -> Option<usize> {
    let limit = read.len().min(BEFORE_INSERT_MAX);
    if let Some(end) = (MOSAIC_END.len()..=limit)
        .find(|&end| differences(&read[end - MOSAIC_END.len()..end], MOSAIC_END) <= 1)
    {
        return Some(end);
    }
    let shortest = read.len().min(MOSAIC_END.len() - 1);
    (MIN_READ_THROUGH..=shortest)
        .rev()
        .find(|&cut| read[..cut] == MOSAIC_END[MOSAIC_END.len() - cut..])
}

/// Une molécule d'un pool : sa séquence et le rang de son échantillon dans la bibliothèque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolMolecule {
    pub sample: usize,
    pub sequence: Vec<u8>,
}

/// Relit un pool écrit avec [`Library::molecule_header`] : la bibliothèque et ses molécules.
pub fn read_pool(records: &[crate::fasta::Record]) -> Result<(Library, Vec<PoolMolecule>)> {
    if records.is_empty() {
        return Err(invalid("le pool ne contient aucune molécule"));
    }
    let mut samples: Vec<Sample> = Vec::new();
    let mut read: Vec<(String, Vec<u8>)> = Vec::with_capacity(records.len());
    for (rank, record) in records.iter().enumerate() {
        let bad = |what: &str| {
            invalid(format!(
                "molécule {rank} : en-tête {what} (attendu : sample=<nom> i7=<nom>:<bases> \
                 i5=<nom>:<bases>)"
            ))
        };
        let field = |key: String| {
            record
                .header
                .split_whitespace()
                .find_map(|token| token.strip_prefix(key.as_str()))
        };
        let index = |position: &str| -> Result<Index> {
            let value = field(format!("{position}="))
                .ok_or_else(|| bad(&format!("sans index {position}")))?;
            let (index_name, bases) = value
                .split_once(':')
                .ok_or_else(|| bad(&format!("d'index {position} mal formé")))?;
            Index::new(index_name, bases)
                .map_err(|error| invalid(format!("molécule {rank} : {error}")))
        };
        let name = field("sample=".to_string())
            .ok_or_else(|| bad("sans échantillon"))?
            .to_string();
        let i5 = match field("i5=".to_string()) {
            Some(_) => Some(index("i5")?),
            None => None,
        };
        let sample = Sample {
            name: name.clone(),
            i7: index("i7")?,
            i5,
        };
        match samples.iter().position(|known| known.name == sample.name) {
            Some(found) if samples[found] != sample => {
                return Err(invalid(format!("deux définitions de l'échantillon {name}")));
            }
            Some(_) => {}
            None => samples.push(sample),
        }
        read.push((name, record.sequence.clone()));
    }
    let library = Library::new(samples)?;
    let mut molecules = Vec::with_capacity(read.len());
    for (rank, (name, sequence)) in read.into_iter().enumerate() {
        let sample = library.rank(&name).unwrap_or(0);
        let (before, after) = library.primer_offsets(sample)?;
        if !sequence.starts_with(P5) || sequence.len() <= before + after {
            return Err(invalid(format!(
                "molécule {rank} : {} bases ; un pool est fait de molécules complètes, \
                 d'adaptateurs et d'index entourant l'insert",
                sequence.len()
            )));
        }
        molecules.push(PoolMolecule { sample, sequence });
    }
    Ok((library, molecules))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, i7: &str, i5: &str) -> Sample {
        Sample {
            name: name.to_string(),
            i5: Some(Index::new(format!("{name}-i5"), i5).unwrap()),
            i7: Index::new(format!("{name}-i7"), i7).unwrap(),
        }
    }

    fn flip(sequence: &[u8]) -> Vec<u8> {
        reverse_complement(sequence)
    }

    fn other(base: u8) -> u8 {
        if base == b'A' {
            b'C'
        } else {
            b'A'
        }
    }

    #[test]
    fn nextera_indexes_are_those_of_the_document() {
        assert_eq!(Index::nextera_i7("N701").unwrap().bases, b"TAAGGCGA");
        assert_eq!(
            Index::nextera_i7("n712").unwrap().describe(),
            "N712=GTAGAGGA"
        );
        assert_eq!(Index::nextera_i5("N501").unwrap().bases, b"TAGATCGC");
        assert_eq!(Index::nextera_i5(" N508 ").unwrap().bases, b"CTAAGCCT");
        assert!(Index::nextera_i7("N713").is_none());
        assert!(Index::nextera_i5("N509").is_none());
        assert!(Index::nextera_i7("S501").is_none());
        // Deux index d'un même kit s'écartent d'au moins trois bases : une erreur ne les confond
        // pas. Et aucun n'est le complément inversé d'un autre.
        for table in [NEXTERA_I7.as_slice(), NEXTERA_I5.as_slice()] {
            let indexes: Vec<Index> = table
                .iter()
                .map(|(name, bases)| Index::new(*name, bases).unwrap())
                .collect();
            for first in &indexes {
                for second in &indexes {
                    if first.name != second.name {
                        assert!(differences(&first.bases, &second.bases) >= MIN_INDEX_DISTANCE);
                        assert!(differences(&flip(&first.bases), &second.bases) > 0);
                    }
                }
            }
        }
        // Dans l'autre sens de lecture, en revanche, deux paires du kit ne sont qu'à deux bases :
        // la bibliothèque les admet, et une lecture équidistante des deux reste sans réponse.
        assert_eq!(differences(&flip(b"CTCTCTAC"), b"GTAGAGGA"), 2);
        assert_eq!(differences(&flip(b"TATCCTCT"), b"AGAGTAGA"), 2);
        assert!(Library::new(vec![
            sample("N707", "CTCTCTAC", "TAGATCGC"),
            sample("N712", "GTAGAGGA", "CTCTCTAT"),
        ])
        .is_ok());
    }

    #[test]
    fn index_sequences_are_checked() {
        assert!(Index::new("x", "ACGTACGTAC").is_ok());
        assert!(Index::new("N901", " acgtacgt ").is_ok());
        assert!(Index::new("x", "ACG").is_err());
        assert!(Index::new("x", "ACGTACGTACGTA").is_err());
        assert!(Index::new("x", "ACGTACGN").is_err());
        assert!(Index::new("x", "").is_err());
        assert_eq!(
            Index::new("N901", " ACGTACGT ").unwrap().describe(),
            "N901=ACGTACGT"
        );
        let err = Index::new("x", "ACGTACGN").unwrap_err();
        assert!(err.to_string().contains("ACGTACGN"), "{err}");
    }

    #[test]
    fn indexes_too_close_are_refused() {
        assert!(Library::new(vec![sample("a", "TAAGGCGA", "TAGATCGC")]).is_ok());
        // Une base de différence sur l'index i7 : une erreur suffirait à confondre.
        let err = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "TAAGGCGT", "CTCTCTAT"),
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("i7 trop proches : b-i7 et a-i7 (1 base(s)"),
            "{err}"
        );
        // Deux bases, sous n'importe quelle forme : encore trop proche.
        for close in ["TAAGAAGA", "TAAGACGT", "TAAGGCTT"] {
            let err = Library::new(vec![
                sample("a", "TAAGGCGA", "TAGATCGC"),
                sample("b", close, "CTCTCTAT"),
            ])
            .unwrap_err();
            assert!(
                err.to_string()
                    .contains("i7 trop proches : b-i7 et a-i7 (2 base(s)"),
                "{err}"
            );
        }
        // Trois bases : assez.
        assert!(Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "TAAGCGTA", "CTCTCTAT"),
        ])
        .is_ok());
        // Même contrôle sur l'index i5.
        let err = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "CGTACTAG", "TAGATCGT"),
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("i5 trop proches : b-i5 et a-i5 (1 base(s)"),
            "{err}"
        );
        // Le complément inversé de l'index de l'un est l'index de l'autre : selon le sens de
        // lecture de la plateforme, une lecture exacte les confondrait.
        let err = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("c", "TCGCCTTA", "TATCCTCT"),
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("i7 : c-i7 est le complément inversé de a-i7"),
            "{err}"
        );
        // Mêmes index, autre nom.
        let err = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "TAAGGCGA", "TAGATCGC"),
        ])
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("i5 trop proches : b-i5 et a-i5 (0 base(s)"),
            "{err}"
        );
        // Longueurs différentes à la même position.
        let err = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "CGTACTAG", "CTCTCTATA"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("même longueur"), "{err}");
        // Même nom.
        let err = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("a", "CGTACTAG", "CTCTCTAT"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("même nom"), "{err}");
        assert!(Library::new(Vec::new()).is_err());
    }

    #[test]
    fn molecules_surround_the_insert_with_adapters_and_indexes() {
        let lib = Library::new(vec![sample("a", "TAAGGCGA", "TAGATCGC")]).unwrap();
        let insert = b"ACGTTGCAACGT";
        let molecule = lib.molecule(0, insert).unwrap();
        assert_eq!(molecule.len(), 29 + 8 + 14 + 19 + 12 + 19 + 15 + 8 + 24);
        assert_eq!(&molecule[..29], P5);
        assert_eq!(&molecule[29..37], b"TAGATCGC");
        assert_eq!(&molecule[37..51], READ1_PRIMER_SITE);
        assert_eq!(&molecule[51..70], MOSAIC_END);
        assert_eq!(&molecule[70..82], insert);
        assert_eq!(&molecule[82..101], READ_THROUGH);
        assert_eq!(&molecule[101..116], READ2_PRIMER_SITE);
        assert_eq!(&molecule[116..124], b"TAAGGCGA");
        assert_eq!(&molecule[124..], flip(P7).as_slice());
        // L'extrémité mosaïque après l'insert est son complément inversé.
        assert_eq!(READ_THROUGH, flip(MOSAIC_END).as_slice());
        // Avec un insert de 117 nt, la molécule fait 253 nt.
        assert_eq!(lib.molecule(0, &[b'A'; 117]).unwrap().len(), 253);
        assert_eq!(lib.primer_offsets(0).unwrap(), (70, 66));
        assert!(lib.molecule(0, &[]).is_err());
        assert!(lib.molecule(1, insert).is_err());
        assert!(lib.primer_offsets(1).is_err());
    }

    #[test]
    fn samples_are_sorted_by_name_and_found_by_name() {
        let lib = Library::new(vec![
            sample("zeta", "CGTACTAG", "CTCTCTAT"),
            sample("alpha", "TAAGGCGA", "TAGATCGC"),
        ])
        .unwrap();
        assert_eq!(lib.samples()[0].name, "alpha");
        assert_eq!(lib.rank("zeta"), Some(1));
        assert_eq!(lib.rank("alpha"), Some(0));
        assert_eq!(lib.rank("bêta"), None);
        assert_eq!(lib.sample(1).i7.bases, b"CGTACTAG");
    }

    #[test]
    fn demultiplexing_tolerates_one_error_per_index_in_either_orientation() {
        let lib = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "CGTACTAG", "CTCTCTAT"),
            sample("c", "AGGCAGAA", "TATCCTCT"),
        ])
        .unwrap();
        for (rank, sample) in lib.samples().iter().enumerate() {
            let exact_i7 = &sample.i7.bases;
            let exact_i5 = &sample.i5.as_ref().unwrap().bases;
            let exact = Assignment::Sample {
                sample: rank,
                mismatches: 0,
            };
            assert_eq!(lib.demultiplex(exact_i7, Some(exact_i5)), exact);
            assert_eq!(lib.demultiplex(exact_i7, None), exact);
            // Index lus dans l'autre sens, selon la plateforme.
            assert_eq!(
                lib.demultiplex(&flip(exact_i7), Some(&flip(exact_i5))),
                exact
            );
            // Une erreur dans chaque index.
            let mut i7 = exact_i7.clone();
            i7[2] = other(i7[2]);
            let mut i5 = exact_i5.clone();
            i5[5] = other(i5[5]);
            assert_eq!(
                lib.demultiplex(&i7, Some(&i5)),
                Assignment::Sample {
                    sample: rank,
                    mismatches: 2
                }
            );
            // Une erreur en tout : attribué.
            assert_eq!(
                lib.demultiplex(&i7, Some(exact_i5)),
                Assignment::Sample {
                    sample: rank,
                    mismatches: 1
                }
            );
            // Deux dans le même index : une lecture dont la longueur a changé, ou trop
            // d'erreurs ; elle n'est attribuée à personne.
            i7[4] = other(i7[4]);
            assert_eq!(lib.demultiplex(&i7, Some(exact_i5)), Assignment::Unknown);
        }
        // Un index i7 qui convient à un échantillon, un index i5 qui en convient à un autre :
        // personne.
        assert_eq!(
            lib.demultiplex(b"TAAGGCGT", Some(b"CTCTCTAT")),
            Assignment::Unknown
        );
        // Un index i5 qui contredit l'index i7.
        assert_eq!(
            lib.demultiplex(b"TAAGGCGA", Some(b"CTCTCTAT")),
            Assignment::Unknown
        );
        // Une lecture d'index tronquée : la base manquante compte pour une erreur.
        assert_eq!(
            lib.demultiplex(b"TAAGGCG", Some(b"TAGATCGC")),
            Assignment::Sample {
                sample: 0,
                mismatches: 1
            }
        );
        assert_eq!(lib.demultiplex(b"ACGTACGT", None), Assignment::Unknown);
    }

    #[test]
    fn an_ambiguous_read_is_left_unless_the_other_index_decides() {
        // N707 et N712 : trois bases les séparent, mais quatre bases d'écart dans un sens de
        // lecture n'en font que deux dans l'autre.
        let lib = Library::new(vec![
            sample("a", "CTCTCTAC", "TAGATCGC"),
            sample("b", "GTAGAGGA", "AGAGTAGA"),
        ])
        .unwrap();
        // À une erreur de N707 lu dans l'autre sens, et de N712 : on ne tranche pas.
        let between = b"GTAGAGGG";
        assert_eq!(lib.demultiplex(between, None), Assignment::Ambiguous);
        // L'index i5 tranche.
        assert_eq!(
            lib.demultiplex(between, Some(b"TAGATCGC")),
            Assignment::Sample {
                sample: 0,
                mismatches: 1
            }
        );
        assert_eq!(
            lib.demultiplex(between, Some(b"AGAGTAGA")),
            Assignment::Sample {
                sample: 1,
                mismatches: 1
            }
        );
        // Un index i5 qui ne convient à aucun des deux.
        assert_eq!(
            lib.demultiplex(between, Some(b"CTAAGCCT")),
            Assignment::Unknown
        );
        assert_eq!(lib.demultiplex(b"ACGTACGT", None), Assignment::Unknown);
        // Une erreur ailleurs dans N712 reste à N712.
        let mut read = b"GTAGAGGA".to_vec();
        read[3] = other(read[3]);
        assert_eq!(
            lib.demultiplex(&read, Some(b"AGAGTAGA")),
            Assignment::Sample {
                sample: 1,
                mismatches: 1
            }
        );
    }

    #[test]
    fn read_through_is_cut_after_and_before_the_insert() {
        let insert = "ACGTTGCAACGTTGCAACGTTGCA";
        let adapter = "CTGTCTCTTATACACATCT";
        let read = format!("{insert}{adapter}CCGAGCCCACGAGACTAAGGCGA");
        assert_eq!(trim_after_insert(read.as_bytes()), insert.as_bytes());
        // Dépassement tronqué : reconnu exactement dès dix bases.
        assert_eq!(
            trim_after_insert(format!("{insert}CTGTCTCTTA").as_bytes()),
            insert.as_bytes()
        );
        // Neuf bases : trop court pour être sûr, la lecture reste telle quelle.
        let short = format!("{insert}CTGTCTCTT");
        assert_eq!(trim_after_insert(short.as_bytes()), short.as_bytes());
        // Une erreur dans un dépassement complet.
        let damaged = adapter.replacen('A', "G", 1);
        assert_ne!(damaged, adapter);
        assert_eq!(
            trim_after_insert(format!("{insert}{damaged}CCGAGCCC").as_bytes()),
            insert.as_bytes()
        );
        // Deux erreurs, ou une erreur dans un dépassement tronqué : rien n'est coupé.
        let twice = adapter.replacen('A', "G", 2);
        let kept = format!("{insert}{twice}CCGAGCCC");
        assert_eq!(trim_after_insert(kept.as_bytes()), kept.as_bytes());
        let kept = format!("{insert}CTGTCTCTTG");
        assert_eq!(trim_after_insert(kept.as_bytes()), kept.as_bytes());
        // Sans dépassement.
        assert_eq!(trim_after_insert(insert.as_bytes()), insert.as_bytes());
        assert_eq!(trim_after_insert(b""), b"");
        // Une ressemblance fortuite de moins de dix bases ne coupe rien.
        let kept = format!("{insert}TGTCTCTTA");
        assert_eq!(trim_after_insert(kept.as_bytes()), kept.as_bytes());
        // Une lecture 2, complément inversée, arrêtée dans l'extrémité mosaïque : il n'en reste
        // que la fin, dix bases ou plus.
        assert_eq!(trim_after_insert(b"ATACACATCT"), b"");
        assert_eq!(trim_after_insert(b"TACACATCT"), b"TACACATCT");

        // Avant l'insert : une lecture 2 qui a traversé l'insert, complément inversée. Elle se
        // termine par l'extrémité mosaïque, précédée de ce qui l'entoure dans la molécule.
        let head = "CCGTTGCAACGTTGCAACGTTGCA";
        let mosaic = "AGATGTGTATAAGAGACAG";
        let before = "AATGATACGGCGACCACCGAGATCTACACTAGATCGCTCGTCGGCAGCGTC";
        assert_eq!(
            trim_before_insert(format!("{before}{mosaic}{head}").as_bytes()),
            head.as_bytes()
        );
        assert_eq!(
            trim_before_insert(format!("{mosaic}{head}").as_bytes()),
            head.as_bytes()
        );
        // L'extrémité entière, à une erreur près.
        let damaged = mosaic.replacen('A', "C", 1);
        assert_ne!(damaged, mosaic);
        assert_eq!(
            trim_before_insert(format!("{damaged}{head}").as_bytes()),
            head.as_bytes()
        );
        // Deux erreurs : rien n'est coupé.
        let twice = mosaic.replacen('A', "C", 2);
        let kept = format!("{before}{twice}{head}");
        assert_eq!(trim_before_insert(kept.as_bytes()), kept.as_bytes());
        // Dépassement plus court que l'extrémité : il en est la fin, reconnue exactement dès dix
        // bases.
        assert_eq!(
            trim_before_insert(format!("GTATAAGAGACAG{head}").as_bytes()),
            head.as_bytes()
        );
        // Neuf bases : trop court pour être sûr.
        let kept = format!("AAGAGACAG{head}");
        assert_eq!(trim_before_insert(kept.as_bytes()), kept.as_bytes());
        // Une lecture réduite à la fin de l'extrémité : vide.
        assert_eq!(trim_before_insert(mosaic.as_bytes()), b"");
        assert_eq!(trim_before_insert(b"TAAGAGACAG"), b"");
        // Sans dépassement, ni extrémité : rien n'est coupé.
        assert_eq!(trim_before_insert(head.as_bytes()), head.as_bytes());
        assert_eq!(trim_before_insert(b""), b"");
    }

    #[test]
    fn a_pool_reads_back_as_a_library_and_its_molecules() {
        let lib = Library::new(vec![
            sample("zeta", "CGTACTAG", "CTCTCTAT"),
            sample("alpha", "TAAGGCGA", "TAGATCGC"),
        ])
        .unwrap();
        let records = vec![
            crate::fasta::Record {
                header: lib.molecule_header(0).unwrap(),
                sequence: lib.molecule(0, &[b'A'; 117]).unwrap(),
            },
            crate::fasta::Record {
                header: lib.molecule_header(1).unwrap(),
                sequence: lib.molecule(1, &[b'C'; 152]).unwrap(),
            },
            crate::fasta::Record {
                header: lib.molecule_header(0).unwrap(),
                sequence: lib.molecule(0, &[b'G'; 117]).unwrap(),
            },
        ];
        assert_eq!(
            records[1].header,
            "sample=zeta i7=zeta-i7:CGTACTAG i5=zeta-i5:CTCTCTAT"
        );
        let (pool, molecules) = read_pool(&records).unwrap();
        assert_eq!(pool, lib);
        assert_eq!(molecules.len(), 3);
        // Les échantillons sont rangés par nom : alpha en premier.
        assert_eq!(molecules[0].sample, 0);
        assert_eq!(molecules[1].sample, 1);
        assert_eq!(molecules[0].sequence, records[0].sequence);

        // En-têtes incomplets ou mal formés.
        for header in [
            "molecule=0",
            "sample=alpha i7=alpha-i7:TAAGGCGA i5=alpha-i5",
            "sample=alpha i7=alpha-i7:TAAGGCGN i5=alpha-i5:TAGATCGC",
        ] {
            let damaged = vec![crate::fasta::Record {
                header: header.to_string(),
                sequence: records[0].sequence.clone(),
            }];
            let err = read_pool(&damaged).unwrap_err();
            assert!(err.to_string().contains("molécule 0"), "{header} : {err}");
        }
        // Deux définitions d'un même échantillon.
        let conflicting = vec![
            records[1].clone(),
            crate::fasta::Record {
                header: "sample=zeta i7=other:AGGCAGAA i5=zeta-i5:CTCTCTAT".to_string(),
                sequence: records[0].sequence.clone(),
            },
        ];
        assert!(read_pool(&conflicting).is_err());
        // Une archive nue n'est pas un pool.
        let bare = vec![crate::fasta::Record {
            header: lib.molecule_header(0).unwrap(),
            sequence: vec![b'A'; 117],
        }];
        let err = read_pool(&bare).unwrap_err();
        assert!(err.to_string().contains("molécules complètes"), "{err}");
        assert!(read_pool(&[]).is_err());
    }

    #[test]
    fn names_must_fit_in_a_pool_header() {
        for name in ["", "deux mots", "a=b", "a:b"] {
            let err = Library::new(vec![Sample {
                name: name.to_string(),
                i5: Some(Index::nextera_i5("N501").unwrap()),
                i7: Index::nextera_i7("N701").unwrap(),
            }])
            .unwrap_err();
            assert!(
                err.to_string().contains("nom d'échantillon invalide"),
                "{err}"
            );
        }
        for name in ["", "deux mots", "a=b", "a:b"] {
            let err = Index::new(name, "TAAGGCGA").unwrap_err();
            assert!(err.to_string().contains("nom invalide"), "{err}");
        }
    }

    #[test]
    fn a_single_index_library_demultiplexes_on_i7_alone() {
        let single = |name: &str, i7: &str| Sample {
            name: name.to_string(),
            i5: None,
            i7: Index::new(format!("{name}-i7"), i7).unwrap(),
        };
        let lib = Library::new(vec![single("a", "TAAGGCGA"), single("b", "CGTACTAG")]).unwrap();
        assert_eq!(
            lib.demultiplex(b"TAAGGCGA", None),
            Assignment::Sample {
                sample: 0,
                mismatches: 0
            }
        );
        // Une lecture i5 est ignorée : la bibliothèque n'en a pas.
        assert_eq!(
            lib.demultiplex(b"CGTACTAG", Some(b"ACGTACGT")),
            Assignment::Sample {
                sample: 1,
                mismatches: 0
            }
        );
        // Un échantillon sur deux sans index i5 : incohérent.
        let err = Library::new(vec![
            single("a", "TAAGGCGA"),
            sample("b", "CGTACTAG", "CTCTCTAT"),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("index i5"), "{err}");
        // Sans index i5, pas de molécule : les index ne se posent que par paires.
        let err = lib.molecule(0, b"ACGT").unwrap_err();
        assert!(err.to_string().contains("deux index"), "{err}");
        assert_eq!(lib.molecule_header(0).unwrap(), "sample=a i7=a-i7:TAAGGCGA");
        // Un pool à un seul index : l'insert commence après le site d'amorce, sans index i5.
        let mut sequence = P5.to_vec();
        for part in [
            READ1_PRIMER_SITE,
            MOSAIC_END,
            b"ACGTACGT".as_slice(),
            READ_THROUGH,
            READ2_PRIMER_SITE,
            b"TAAGGCGA".as_slice(),
            &reverse_complement(P7),
        ] {
            sequence.extend_from_slice(part);
        }
        assert_eq!(lib.primer_offsets(0).unwrap(), (62, 66));
        let pool = vec![crate::fasta::Record {
            header: lib.molecule_header(0).unwrap(),
            sequence: sequence.clone(),
        }];
        let (reread, molecules) = read_pool(&pool).unwrap();
        // Le pool ne contient que l'échantillon a.
        assert_eq!(reread.samples(), &lib.samples()[..1]);
        assert_eq!(molecules[0].sequence, sequence);
    }

    #[test]
    fn a_pair_of_reads_is_placed_and_trimmed() {
        let lib = Library::new(vec![
            sample("a", "TAAGGCGA", "TAGATCGC"),
            sample("b", "CGTACTAG", "CTCTCTAT"),
        ])
        .unwrap();
        let insert = b"ACGTTGCAACGTTGCAACGTTGCAACGTTGCAACGTTGCA";
        let molecule = lib.molecule(0, insert).unwrap();
        let (before, _) = lib.primer_offsets(0).unwrap();
        // Lectures telles que le séquenceur les tire, jusqu'au bout de la molécule.
        let r1 = molecule[before..].to_vec();
        let r2 = reverse_complement(&molecule[..before + insert.len() + READ_THROUGH.len()]);
        let i7 = reverse_complement(b"TAAGGCGA");
        let placed = || match place(&lib, &r1, &r2, &i7, Some(b"TAGATCGC")) {
            Placement::Sample(reads) => reads,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            placed(),
            InsertReads {
                sample: 0,
                r1: insert.to_vec(),
                r2: insert.to_vec(),
                r2_partial: false,
            }
        );
        // Sans lecture d'index i5 : rangé sur le seul index i7.
        assert!(matches!(
            place(&lib, &r1, &r2, &i7, None),
            Placement::Sample(_)
        ));
        // Index d'un autre échantillon, lecture i5 de celui-ci : personne.
        assert_eq!(
            place(
                &lib,
                &r1,
                &r2,
                &reverse_complement(b"CGTACTAG"),
                Some(b"TAGATCGC")
            ),
            Placement::Unknown
        );
        // Lectures plus courtes que l'insert : ce que chacune en a lu.
        let placed_with = |r1: &[u8], r2: &[u8]| match place(&lib, r1, r2, &i7, None) {
            Placement::Sample(reads) => reads,
            other => panic!("{other:?}"),
        };
        let r1 = molecule[before..before + 20].to_vec();
        let r2 = [MOSAIC_END, reverse_complement(&insert[20..]).as_slice()].concat();
        let reads = placed_with(&r1, &r2);
        assert_eq!(reads.r1, insert[..20]);
        assert_eq!(reads.r2, insert[20..]);
        assert!(!reads.r2_partial);
        // Une lecture 2 arrêtée dans l'extrémité mosaïque : vide.
        let r2 = MOSAIC_END[..10].to_vec();
        assert!(placed_with(&r1, &r2).r2.is_empty());
        // Une lecture 1 qui dépasse l'insert de moins de dix bases : rien n'est reconnaissable,
        // elle reste telle quelle.
        let r1 = molecule[before..before + insert.len() + 5].to_vec();
        assert_eq!(placed_with(&r1, &r2).r1, r1);
        // La lecture 1 lit l'insert en entier, la lecture 2 s'arrête dedans : il n'en reste que
        // la fin, qu'aucun schéma ne lit. Elle est écartée.
        let r1 = molecule[before..].to_vec();
        let r2 = [MOSAIC_END, reverse_complement(&insert[20..]).as_slice()].concat();
        let reads = placed_with(&r1, &r2);
        assert_eq!(reads.r1, insert);
        assert!(reads.r2.is_empty());
        assert!(reads.r2_partial);
        // Même chose quand la lecture 1 s'arrête pile au bout de l'insert : un séquençage
        // 2 × 300 sur un brin de 300 nt.
        let r1 = molecule[before..before + insert.len()].to_vec();
        assert!(placed_with(&r1, &r2).r2_partial);
    }
}
