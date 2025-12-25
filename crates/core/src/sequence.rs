//! Structures de séquences ADN et métadonnées

use crate::error::{DnaError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

/// Codes IUPAC pour les nucléotides
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IupacBase {
    A,  // Adénine
    C,  // Cytosine
    G,  // Guanine
    T,  // Thymine
    R,  // A ou G (purine)
    Y,  // C ou T (pyrimidine)
    S,  // G ou C (strong)
    W,  // A ou T (weak)
    K,  // G ou T (keto)
    M,  // A ou C (amino)
    B,  // C ou G ou T
    D,  // A ou G ou T
    H,  // A ou C ou T
    V,  // A ou C ou G
    N,  // Any base
}

impl IupacBase {
    /// Convertit un caractère en base IUPAC
    pub fn from_char(c: char) -> Result<Self> {
        match c.to_ascii_uppercase() {
            'A' => Ok(IupacBase::A),
            'C' => Ok(IupacBase::C),
            'G' => Ok(IupacBase::G),
            'T' => Ok(IupacBase::T),
            'R' => Ok(IupacBase::R),
            'Y' => Ok(IupacBase::Y),
            'S' => Ok(IupacBase::S),
            'W' => Ok(IupacBase::W),
            'K' => Ok(IupacBase::K),
            'M' => Ok(IupacBase::M),
            'B' => Ok(IupacBase::B),
            'D' => Ok(IupacBase::D),
            'H' => Ok(IupacBase::H),
            'V' => Ok(IupacBase::V),
            'N' => Ok(IupacBase::N),
            _ => Err(DnaError::InvalidBase(c)),
        }
    }

    /// Convertit une base en caractère
    pub fn as_char(self) -> char {
        match self {
            IupacBase::A => 'A',
            IupacBase::C => 'C',
            IupacBase::G => 'G',
            IupacBase::T => 'T',
            IupacBase::R => 'R',
            IupacBase::Y => 'Y',
            IupacBase::S => 'S',
            IupacBase::W => 'W',
            IupacBase::K => 'K',
            IupacBase::M => 'M',
            IupacBase::B => 'B',
            IupacBase::D => 'D',
            IupacBase::H => 'H',
            IupacBase::V => 'V',
            IupacBase::N => 'N',
        }
    }

    /// Vérifie si c'est une base standard (non ambiguë)
    pub fn is_standard(self) -> bool {
        matches!(self, IupacBase::A | IupacBase::C | IupacBase::G | IupacBase::T)
    }

    /// Retourne true si c'est une base GC
    pub fn is_gc(self) -> bool {
        matches!(self, IupacBase::G | IupacBase::C | IupacBase::S | IupacBase::B | IupacBase::V)
    }
}

impl fmt::Display for IupacBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_char())
    }
}

impl TryFrom<char> for IupacBase {
    type Error = DnaError;

    fn try_from(c: char) -> Result<Self> {
        IupacBase::from_char(c)
    }
}

/// Identifiant unique de séquence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SequenceId(Uuid);

impl SequenceId {
    /// Génère un nouvel ID de séquence
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    /// Crée un ID depuis un UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Retourne l'UUID sous-jacent
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for SequenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SequenceId {
    fn default() -> Self {
        Self::generate()
    }
}

/// Métadonnées de séquence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceMetadata {
    /// Timestamp de création
    pub timestamp: DateTime<Utc>,
    /// Nom du fichier original
    pub original_file: String,
    /// Index du chunk
    pub chunk_index: usize,
    /// Taille du chunk
    pub chunk_size: usize,
    /// Ratio GC (0-1)
    pub gc_ratio: f64,
    /// Longueur max d'homopolymer
    pub max_homopolymer: usize,
    /// Entropie de Shannon
    pub entropy: f64,
    /// Checksum SHA-256
    pub checksum: String,
    /// Seed utilisé pour la génération
    pub seed: u64,
    /// Schéma d'encodage utilisé
    pub encoding_scheme: String,
}

impl SequenceMetadata {
    /// Calcule les métadonnées pour une séquence
    pub fn compute(
        bases: &[IupacBase],
        original_file: String,
        chunk_index: usize,
        chunk_size: usize,
        seed: u64,
        encoding_scheme: String,
    ) -> Self {
        // Calcul du ratio GC
        let gc_count = bases.iter().filter(|b| b.is_gc()).count();
        let gc_ratio = gc_count as f64 / bases.len() as f64;

        // Calcul du max homopolymer
        let mut max_homopolymer = 0;
        let mut current_run = 0;
        let mut last_base: Option<IupacBase> = None;

        for base in bases {
            if Some(*base) == last_base {
                current_run += 1;
            } else {
                max_homopolymer = max_homopolymer.max(current_run);
                current_run = 1;
                last_base = Some(*base);
            }
        }
        max_homopolymer = max_homopolymer.max(current_run);

        // Calcul de l'entropie de Shannon
        let mut freq = [0usize; 4];
        for base in bases {
            match base {
                IupacBase::A => freq[0] += 1,
                IupacBase::C => freq[1] += 1,
                IupacBase::G => freq[2] += 1,
                IupacBase::T => freq[3] += 1,
                _ => {}
            }
        }

        let len = bases.len() as f64;
        let entropy: f64 = freq
            .iter()
            .filter(|&&c| c > 0)
            .map(|&c| {
                let p = c as f64 / len;
                -p * p.log2()
            })
            .sum();

        // Calcul du checksum
        let sequence_str: String = bases.iter().map(|b| b.as_char()).collect();
        let hash = Sha256::digest(sequence_str.as_bytes());
        let checksum = format!("{:x}", hash);

        Self {
            timestamp: Utc::now(),
            original_file,
            chunk_index,
            chunk_size,
            gc_ratio,
            max_homopolymer,
            entropy,
            checksum,
            seed,
            encoding_scheme,
        }
    }
}

/// Contraintes ADN configurables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnaConstraints {
    /// Ratio GC minimum (0-1)
    pub gc_min: f64,
    /// Ratio GC maximum (0-1)
    pub gc_max: f64,
    /// Longueur max d'homopolymer
    pub max_homopolymer: usize,
    /// Longueur max de séquence
    pub max_sequence_length: usize,
    /// Bases autorisées
    pub allowed_bases: Vec<IupacBase>,
}

impl Default for DnaConstraints {
    fn default() -> Self {
        Self {
            gc_min: 0.40,
            gc_max: 0.60,
            max_homopolymer: 3,
            max_sequence_length: 150, // Standard Illumina
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        }
    }
}

impl DnaConstraints {
    /// Crée des contraintes avec des valeurs personnalisées
    pub fn new(gc_min: f64, gc_max: f64, max_homopolymer: usize, max_length: usize) -> Self {
        Self {
            gc_min,
            gc_max,
            max_homopolymer,
            max_sequence_length: max_length,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        }
    }

    /// Valide une séquence selon les contraintes
    pub fn validate(&self, bases: &[IupacBase]) -> Result<()> {
        // Vérifier la longueur
        if bases.len() > self.max_sequence_length {
            return Err(DnaError::SequenceTooLong {
                len: bases.len(),
                max: self.max_sequence_length,
            });
        }

        // Vérifier le GC content
        let gc_count = bases.iter().filter(|b| b.is_gc()).count();
        let gc_ratio = gc_count as f64 / bases.len() as f64;

        if gc_ratio < self.gc_min || gc_ratio > self.gc_max {
            return Err(DnaError::GcContentOutOfRange {
                gc: gc_ratio,
                min: self.gc_min,
                max: self.gc_max,
            });
        }

        // Vérifier les homopolymers
        let mut current_run = 0;
        let mut last_base: Option<IupacBase> = None;

        for base in bases {
            if Some(*base) == last_base {
                current_run += 1;
                if current_run > self.max_homopolymer {
                    return Err(DnaError::HomopolymerRun {
                        base: base.as_char(),
                        count: current_run,
                    });
                }
            } else {
                current_run = 1;
                last_base = Some(*base);
            }
        }

        // Vérifier les bases autorisées
        for base in bases {
            if !self.allowed_bases.contains(base) {
                return Err(DnaError::InvalidBase(base.as_char()));
            }
        }

        Ok(())
    }
}

/// Séquence ADN avec métadonnées
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnaSequence {
    /// Bases de la séquence
    pub bases: Vec<IupacBase>,
    /// ID unique
    pub id: SequenceId,
    /// Métadonnées
    pub metadata: SequenceMetadata,
}

impl DnaSequence {
    /// Crée une nouvelle séquence ADN
    pub fn new(
        bases: Vec<IupacBase>,
        original_file: String,
        chunk_index: usize,
        chunk_size: usize,
        seed: u64,
    ) -> Self {
        let metadata = SequenceMetadata::compute(
            &bases,
            original_file,
            chunk_index,
            chunk_size,
            seed,
            "unknown".to_string(), // Default encoding scheme
        );

        Self {
            bases,
            id: SequenceId::generate(),
            metadata,
        }
    }

    /// Crée une nouvelle séquence ADN avec un schéma d'encodage spécifié
    pub fn with_encoding_scheme(
        bases: Vec<IupacBase>,
        original_file: String,
        chunk_index: usize,
        chunk_size: usize,
        seed: u64,
        encoding_scheme: String,
    ) -> Self {
        let metadata = SequenceMetadata::compute(
            &bases,
            original_file,
            chunk_index,
            chunk_size,
            seed,
            encoding_scheme,
        );

        Self {
            bases,
            id: SequenceId::generate(),
            metadata,
        }
    }

    /// Valide la séquence selon des contraintes
    pub fn validate(&self, constraints: &DnaConstraints) -> Result<()> {
        constraints.validate(&self.bases)
    }

    /// Convertit au format FASTA
    pub fn to_fasta(&self) -> String {
        format!(
            ">{}|scheme:{}|seed:{}|gc:{:.2}|len:{}\n{}\n",
            self.id,
            self.metadata.encoding_scheme,
            self.metadata.seed,
            self.metadata.gc_ratio * 100.0,
            self.bases.len(),
            self
        )
    }

    /// Parse une séquence depuis une ligne FASTA
    pub fn from_fasta(fasta: &str) -> Result<Self> {
        let lines: Vec<&str> = fasta.trim().lines().collect();

        if lines.is_empty() {
            return Err(DnaError::Decoding("Fasta vide".to_string()));
        }

        // Parser l'en-tête
        let header = lines[0];
        if !header.starts_with('>') {
            return Err(DnaError::Decoding("Format FASTA invalide: pas d'en-tête >".to_string()));
        }

        // Extraire les métadonnées depuis l'en-tête
        let metadata_parts = header[1..].split('|').collect::<Vec<_>>();
        let mut scheme = "unknown".to_string();
        let mut seed = 0u64;

        for part in metadata_parts {
            if part.contains("scheme:") {
                scheme = part.split(':').nth(1).unwrap_or("unknown").to_string();
            } else if part.contains("seed:") {
                let seed_str = part.split(':').nth(1).unwrap_or("0");
                seed = seed_str.parse().unwrap_or(0);
            }
        }

        // Parser les bases
        let sequence_data = lines[1..].join("");
        let bases = sequence_data
            .chars()
            .map(IupacBase::from_char)
            .collect::<Result<Vec<IupacBase>>>()?;

        // Créer les métadonnées
        let metadata = SequenceMetadata::compute(
            &bases,
            String::from("fasta"),
            0,
            bases.len(),
            seed,
            scheme,
        );

        Ok(Self {
            bases,
            id: SequenceId::generate(),
            metadata,
        })
    }

    /// Retourne la longueur de la séquence
    pub fn len(&self) -> usize {
        self.bases.len()
    }

    /// Vérifie si la séquence est vide
    pub fn is_empty(&self) -> bool {
        self.bases.is_empty()
    }

    /// Parse une séquence depuis une chaîne
    pub fn from_str(
        s: &str,
        original_file: String,
        chunk_index: usize,
        chunk_size: usize,
        seed: u64,
    ) -> Result<Self> {
        let bases: Result<Vec<IupacBase>> = s.chars().map(IupacBase::from_char).collect();
        let bases = bases?;

        Ok(Self::new(bases, original_file, chunk_index, chunk_size, seed))
    }
}

impl fmt::Display for DnaSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for base in &self.bases {
            write!(f, "{}", base.as_char())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iupac_base_conversion() {
        assert_eq!(IupacBase::from_char('A').unwrap(), IupacBase::A);
        assert_eq!(IupacBase::from_char('c').unwrap(), IupacBase::C);
        assert!(IupacBase::from_char('X').is_err());
    }

    #[test]
    fn test_gc_content() {
        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let metadata = SequenceMetadata::compute(&bases, "test.txt".to_string(), 0, 4, 0, "test".to_string());

        assert_eq!(metadata.gc_ratio, 0.5);
    }

    #[test]
    fn test_homopolymer_detection() {
        let bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
        ];
        let metadata = SequenceMetadata::compute(&bases, "test.txt".to_string(), 0, 5, 0, "test".to_string());

        assert_eq!(metadata.max_homopolymer, 3);
    }

    #[test]
    fn test_constraints_validation() {
        let constraints = DnaConstraints::default();

        // Séquence valide
        let valid_bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
            IupacBase::A,
            IupacBase::C,
        ];
        assert!(constraints.validate(&valid_bases).is_ok());

        // Homopolymer trop long
        let invalid_bases = vec![
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
            IupacBase::A,
        ];
        assert!(constraints.validate(&invalid_bases).is_err());
    }

    #[test]
    fn test_dna_sequence_creation() {
        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases.clone(), "test.txt".to_string(), 0, 4, 42);

        assert_eq!(seq.bases, bases);
        assert_eq!(seq.len(), 4);
        assert!(!seq.is_empty());
        assert_eq!(seq.to_string(), "ACGT");
    }

    #[test]
    fn test_fasta_format() {
        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let fasta = seq.to_fasta();
        assert!(fasta.starts_with('>'));
        assert!(fasta.contains("seed:42"));
        assert!(fasta.contains("ACGT"));
    }
}
