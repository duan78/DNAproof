//! Standards Illumina pour le séquencement ADN
//!
//! Ce module implémente les standards Illumina utilisés dans le séquencement
//! moderne: barcodes (index), adapters (P5/P7), et validation de séquences.

use crate::error::{DnaError, Result};
use crate::sequence::{DnaSequence, IupacBase};
use serde::{Deserialize, Serialize};

/// Barcode Illumina (index pour multiplexing)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IlluminaBarcode {
    /// Séquence du barcode
    pub sequence: Vec<IupacBase>,
    /// Nom/index du barcode
    pub index: String,
    /// Position du barcode (5' ou 3')
    pub position: BarcodePosition,
}

/// Position du barcode dans la séquence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarcodePosition {
    FivePrime,
    ThreePrime,
}

impl IlluminaBarcode {
    /// Crée un nouveau barcode
    pub fn new(sequence: Vec<IupacBase>, index: String, position: BarcodePosition) -> Self {
        Self {
            sequence,
            index,
            position,
        }
    }

    /// Retourne la longueur du barcode
    pub fn len(&self) -> usize {
        self.sequence.len()
    }

    /// Retourne true si le barcode est vide
    pub fn is_empty(&self) -> bool {
        self.sequence.is_empty()
    }

    /// Barcodes Illumina standards (Nextera/i5/i7)
    pub fn standard_barcodes() -> Vec<Self> {
        // Ces barcodes sont des exemples de barcodes Illumina courants
        // Les vrais barcodes dépendent du kit utilisé
        vec![
            Self::new(
                vec![IupacBase::A, IupacBase::T, IupacBase::G, IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::C, IupacBase::A],
                "N701".to_string(),
                BarcodePosition::FivePrime,
            ),
            Self::new(
                vec![IupacBase::C, IupacBase::G, IupacBase::T, IupacBase::A, IupacBase::G, IupacBase::C, IupacBase::T, IupacBase::A],
                "N702".to_string(),
                BarcodePosition::FivePrime,
            ),
        ]
    }
}

/// Adapter Illumina (amorce de séquencement)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IlluminaAdapter {
    /// Séquence de l'adapter
    pub sequence: Vec<IupacBase>,
    /// Type d'adapter
    pub adapter_type: AdapterType,
}

/// Type d'adapter Illumina
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterType {
    /// Adapter P5 (5' primer binding site)
    P5,
    /// Adapter P7 (3' primer binding site)
    P7,
    /// Adapter personnalisé
    Custom(String),
}

impl IlluminaAdapter {
    /// Crée un nouvel adapter
    pub fn new(sequence: Vec<IupacBase>, adapter_type: AdapterType) -> Self {
        Self {
            sequence,
            adapter_type,
        }
    }

    /// Adapter P5 standard Illumina
    pub fn standard_p5() -> Self {
        Self::new(
            vec![
                IupacBase::A, IupacBase::A, IupacBase::T, IupacBase::G,
                IupacBase::A, IupacBase::T, IupacBase::C, IupacBase::G,
                IupacBase::G, IupacBase::A, IupacBase::G, IupacBase::A,
                // P5 est plus long en réalité, version simplifiée
            ],
            AdapterType::P5,
        )
    }

    /// Adapter P7 standard Illumina
    pub fn standard_p7() -> Self {
        Self::new(
            vec![
                IupacBase::C, IupacBase::A, IupacBase::A, IupacBase::G,
                IupacBase::C, IupacBase::A, IupacBase::G, IupacBase::A,
                IupacBase::C, IupacBase::G, IupacBase::A, IupacBase::C,
                // P7 est plus long en réalité, version simplifiée
            ],
            AdapterType::P7,
        )
    }

    /// Retourne la longueur de l'adapter
    pub fn len(&self) -> usize {
        self.sequence.len()
    }

    /// Retourne true si l'adapter est vide
    pub fn is_empty(&self) -> bool {
        self.sequence.is_empty()
    }
}

/// Configuration du système Illumina
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminaConfig {
    /// Barcodes à utiliser
    pub barcodes: Vec<IlluminaBarcode>,
    /// Adapters à utiliser
    pub adapters: Vec<IlluminaAdapter>,
    /// Longueur cible des séquences
    pub target_length: usize,
    /// GC-content minimum
    pub min_gc: f64,
    /// GC-content maximum
    pub max_gc: f64,
}

impl Default for IlluminaConfig {
    fn default() -> Self {
        Self {
            barcodes: IlluminaBarcode::standard_barcodes(),
            adapters: vec![IlluminaAdapter::standard_p5(), IlluminaAdapter::standard_p7()],
            target_length: 150, // Standard Illumina
            min_gc: 0.40,
            max_gc: 0.60,
        }
    }
}

/// Système complet Illumina pour l'indexing
pub struct IlluminaSystem {
    config: IlluminaConfig,
    validator: IlluminaValidator,
}

impl IlluminaSystem {
    /// Crée un nouveau système Illumina
    pub fn new(config: IlluminaConfig) -> Self {
        let validator = IlluminaValidator::from_config(&config);
        Self { config, validator }
    }

    /// Crée un système Illumina avec la configuration par défaut
    pub fn default_system() -> Self {
        Self::new(IlluminaConfig::default())
    }

    /// Ajoute barcode et adapters aux séquences
    ///
    /// # Format résultant
    /// [Adapter P5] [Barcode 5'] [Données] [Barcode 3'] [Adapter P7]
    pub fn add_indexing(&self, sequences: &[DnaSequence]) -> Result<Vec<DnaSequence>> {
        let mut indexed_sequences = Vec::new();

        for seq in sequences {
            // Trouver le barcode P5 et P7
            let p5_barcode = self.config.barcodes.iter()
                .find(|b| b.position == BarcodePosition::FivePrime);

            let p7_barcode = self.config.barcodes.iter()
                .find(|b| b.position == BarcodePosition::ThreePrime);

            // Trouver les adapters
            let p5_adapter = self.config.adapters.iter()
                .find(|a| a.adapter_type == AdapterType::P5);

            let p7_adapter = self.config.adapters.iter()
                .find(|a| a.adapter_type == AdapterType::P7);

            // Construire la nouvelle séquence
            let mut new_bases = Vec::new();

            // Ajouter P5 adapter si disponible
            if let Some(adapter) = p5_adapter {
                new_bases.extend(adapter.sequence.clone());
            }

            // Ajouter P5 barcode si disponible
            if let Some(barcode) = p5_barcode {
                new_bases.extend(barcode.sequence.clone());
            }

            // Ajouter les données originales
            new_bases.extend(seq.bases.clone());

            // Ajouter P7 barcode si disponible
            if let Some(barcode) = p7_barcode {
                new_bases.extend(barcode.sequence.clone());
            }

            // Ajouter P7 adapter si disponible
            if let Some(adapter) = p7_adapter {
                new_bases.extend(adapter.sequence.clone());
            }

            // Créer la nouvelle séquence
            let mut new_seq = seq.clone();
            new_seq.bases = new_bases;

            // Valider
            self.validator.validate(&new_seq)?;

            indexed_sequences.push(new_seq);
        }

        Ok(indexed_sequences)
    }

    /// Retire les barcodes et adapters des séquences
    pub fn remove_indexing(&self, sequences: &[DnaSequence]) -> Result<Vec<DnaSequence>> {
        let mut cleaned_sequences = Vec::new();

        for seq in sequences {
            // Trouver les adapters pour connaître leurs longueurs
            let p5_adapter_len = self.config.adapters.iter()
                .find(|a| a.adapter_type == AdapterType::P5)
                .map(|a| a.len())
                .unwrap_or(0);

            let p5_barcode_len = self.config.barcodes.iter()
                .find(|b| b.position == BarcodePosition::FivePrime)
                .map(|b| b.len())
                .unwrap_or(0);

            let p7_barcode_len = self.config.barcodes.iter()
                .find(|b| b.position == BarcodePosition::ThreePrime)
                .map(|b| b.len())
                .unwrap_or(0);

            let p7_adapter_len = self.config.adapters.iter()
                .find(|a| a.adapter_type == AdapterType::P7)
                .map(|a| a.len())
                .unwrap_or(0);

            let prefix_len = p5_adapter_len + p5_barcode_len;
            let suffix_len = p7_barcode_len + p7_adapter_len;

            // Vérifier que la séquence est assez longue
            if seq.bases.len() <= prefix_len + suffix_len {
                return Err(DnaError::ConstraintViolation(
                    "Séquence trop courte pour contenir des données après retrait des adapters".to_string()
                ));
            }

            // Extraire seulement les données
            let mut new_seq = seq.clone();
            new_seq.bases = seq.bases[prefix_len..seq.bases.len() - suffix_len].to_vec();

            cleaned_sequences.push(new_seq);
        }

        Ok(cleaned_sequences)
    }

    /// Valide une séquence avec les contraintes Illumina
    pub fn validate(&self, seq: &DnaSequence) -> Result<()> {
        self.validator.validate(seq)
    }
}

/// Validateur de contraintes Illumina
pub struct IlluminaValidator {
    min_gc: f64,
    max_gc: f64,
    target_length: usize,
    max_homopolymer: usize,
}

impl IlluminaValidator {
    /// Crée un validateur depuis une configuration
    pub fn from_config(config: &IlluminaConfig) -> Self {
        Self {
            min_gc: config.min_gc,
            max_gc: config.max_gc,
            target_length: config.target_length,
            max_homopolymer: 3, // Standard Illumina
        }
    }

    /// Valide une séquence ADN
    pub fn validate(&self, seq: &DnaSequence) -> Result<()> {
        // Vérifier la longueur
        if seq.bases.len() > self.target_length * 2 {
            return Err(DnaError::ConstraintViolation(format!(
                "Séquence trop longue: {} nt (max {})", seq.bases.len(), self.target_length * 2
            )));
        }

        // Calculer le GC-content
        let gc_ratio = self.calculate_gc(&seq.bases);

        if gc_ratio < self.min_gc || gc_ratio > self.max_gc {
            return Err(DnaError::ConstraintViolation(format!(
                "GC-content invalide: {:.2}% (attendu: {:.0}%-{:.0}%)",
                gc_ratio * 100.0,
                self.min_gc * 100.0,
                self.max_gc * 100.0
            )));
        }

        // Vérifier les homopolymères
        if self.has_long_homopolymer(&seq.bases) {
            return Err(DnaError::ConstraintViolation(
                "Homopolymère de plus de 3 bases détecté".to_string()
            ));
        }

        Ok(())
    }

    /// Calcule le GC-content d'une séquence
    fn calculate_gc(&self, bases: &[IupacBase]) -> f64 {
        if bases.is_empty() {
            return 0.0;
        }

        let gc_count = bases.iter()
            .filter(|b| matches!(b, IupacBase::G | IupacBase::C))
            .count();

        gc_count as f64 / bases.len() as f64
    }

    /// Vérifie si la séquence contient un long homopolymère
    fn has_long_homopolymer(&self, bases: &[IupacBase]) -> bool {
        if bases.len() <= self.max_homopolymer {
            return false;
        }

        for i in 0..=(bases.len() - self.max_homopolymer - 1) {
            let window = &bases[i..i + self.max_homopolymer + 1];
            if window.iter().all(|b| b == &window[0]) {
                return true;
            }
        }

        false
    }

    /// Retourne le GC-content d'une séquence
    pub fn gc_content(&self, seq: &DnaSequence) -> f64 {
        self.calculate_gc(&seq.bases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_barcode_creation() {
        let barcode = IlluminaBarcode::new(
            vec![IupacBase::A, IupacBase::T, IupacBase::G, IupacBase::C],
            "test".to_string(),
            BarcodePosition::FivePrime,
        );

        assert_eq!(barcode.len(), 4);
        assert_eq!(barcode.index, "test");
        assert!(!barcode.is_empty());
    }

    #[test]
    fn test_adapter_creation() {
        let adapter = IlluminaAdapter::standard_p5();

        assert!(!adapter.is_empty());
        assert_eq!(adapter.adapter_type, AdapterType::P5);
    }

    #[test]
    fn test_validator_valid_sequence() {
        let validator = IlluminaValidator::from_config(&IlluminaConfig::default());

        // Créer une séquence alternée avec ~50% GC
        let bases: Vec<IupacBase> = (0..50)
            .map(|i| if i % 2 == 0 { IupacBase::G } else { IupacBase::A })
            .collect();

        let seq = DnaSequence::new(bases, "test".to_string(), 0, 50, 42);

        assert!(validator.validate(&seq).is_ok());
    }

    #[test]
    fn test_validator_invalid_gc() {
        let validator = IlluminaValidator::from_config(&IlluminaConfig::default());

        // Tout A/T -> GC = 0%
        let seq = DnaSequence::new(
            vec![IupacBase::A; 100],
            "test".to_string(),
            0,
            100,
            42,
        );

        assert!(validator.validate(&seq).is_err());
    }

    #[test]
    fn test_validator_homopolymer() {
        let validator = IlluminaValidator::from_config(&IlluminaConfig::default());

        // Homopolymère de 4 A
        let bases = vec![
            IupacBase::A, IupacBase::A, IupacBase::A, IupacBase::A,
            IupacBase::C, IupacBase::G, IupacBase::T,
        ];
        let seq = DnaSequence::new(bases, "test".to_string(), 0, 7, 42);

        assert!(validator.validate(&seq).is_err());
    }

    #[test]
    fn test_illumina_system() {
        let system = IlluminaSystem::default_system();

        // Créer une séquence avec ~50% GC
        let bases: Vec<IupacBase> = (0..50)
            .map(|i| if i % 2 == 0 { IupacBase::G } else { IupacBase::A })
            .collect();

        let seq = DnaSequence::new(bases, "test".to_string(), 0, 50, 42);

        // Ajouter indexing
        let indexed = system.add_indexing(&[seq.clone()]).unwrap();

        assert!(!indexed.is_empty());
        assert!(indexed[0].bases.len() > seq.bases.len());

        // Retirer indexing
        let cleaned = system.remove_indexing(&indexed).unwrap();

        assert_eq!(cleaned[0].bases.len(), seq.bases.len());
    }

    #[test]
    fn test_gc_content_calculation() {
        let validator = IlluminaValidator::from_config(&IlluminaConfig::default());

        let seq = DnaSequence::new(
            vec![
                IupacBase::G, IupacBase::C,  // 2 GC
                IupacBase::A, IupacBase::T,  // 2 AT
                IupacBase::G, IupacBase::C,  // 2 GC
                IupacBase::A, IupacBase::T,  // 2 AT
            ],
            "test".to_string(),
            0,
            8,
            42,
        );

        let gc = validator.gc_content(&seq);
        assert!((gc - 0.5).abs() < 0.01); // 50% GC
    }
}
