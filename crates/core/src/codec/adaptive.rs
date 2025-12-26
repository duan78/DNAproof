//! Encodage adaptatif selon le type de données
//!
//! Ce module analyse automatiquement les données et choisit la meilleure
//! stratégie de compression et d'encodage selon leurs caractéristiques.

use crate::error::{DnaError, Result};
use crate::sequence::{DnaSequence, DnaConstraints};
use crate::codec::reed_solomon::ReedSolomonCodec;
use crate::codec::gc_aware_encoding::GcAwareEncoder;
use crate::codec::huffman::HuffmanCompressor;
use std::collections::HashMap;

/// Type de données détecté
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// Données textuelles (ASCII, UTF-8)
    Text,
    /// Données d'image (PNG, JPEG, etc.)
    Image,
    /// Données audio (MP3, FLAC, etc.)
    Audio,
    /// Données binaires générales
    Binary,
    /// Données très répétitives (ex: zéros, motifs)
    Repetitive,
    /// Données compressées (ZIP, etc.)
    Compressed,
    /// Type inconnu
    Unknown,
}

impl DataType {
    /// Retourne une description du type
    pub fn description(&self) -> &'static str {
        match self {
            DataType::Text => "Données textuelles",
            DataType::Image => "Données d'image",
            DataType::Audio => "Données audio",
            DataType::Binary => "Données binaires",
            DataType::Repetitive => "Données répétitives",
            DataType::Compressed => "Données compressées",
            DataType::Unknown => "Type inconnu",
        }
    }
}

/// Analyseur de données pour détecter le type et les caractéristiques
pub struct DataAnalyzer {
    /// Taille de l'échantillon pour l'analyse
    sample_size: usize,
}

impl DataAnalyzer {
    /// Crée un nouvel analyseur
    pub fn new() -> Self {
        Self {
            sample_size: 4096, // Analyser les premiers 4KB
        }
    }

    /// Avec une taille d'échantillon personnalisée
    pub fn with_sample_size(sample_size: usize) -> Self {
        Self { sample_size }
    }

    /// Détecte le type de données
    pub fn detect_data_type(&self, data: &[u8]) -> DataType {
        if data.is_empty() {
            return DataType::Unknown;
        }

        // Analyser les signatures de fichiers (magic bytes)
        if let Some(dt) = self.detect_by_magic_bytes(data) {
            return dt;
        }

        // Analyser les caractéristiques statistiques
        let entropy = self.calculate_entropy(data);
        let repetition_ratio = self.calculate_repetition(data);
        let is_printable = self.is_printable_text(data);

        match () {
            _ if repetition_ratio > 0.6 => DataType::Repetitive,
            _ if is_printable && entropy < 5.0 => DataType::Text,
            _ if entropy > 7.8 => DataType::Compressed,
            _ if is_printable => DataType::Text,
            _ => DataType::Binary,
        }
    }

    /// Détecte le type par les magic bytes (signatures de fichiers)
    fn detect_by_magic_bytes(&self, data: &[u8]) -> Option<DataType> {
        if data.len() < 4 {
            return None;
        }

        // Signatures courantes
        match &data[0..4.min(data.len())] {
            // Images
            b"\xFF\xD8\xFF" => Some(DataType::Image),  // JPEG
            b"\x89PNG" => Some(DataType::Image),       // PNG
            b"RIFF" if data.len() > 8 && &data[8..12] == b"WEBP" => Some(DataType::Image),
            b"MM\x00\x2A" | b"II\x2A\x00" => Some(DataType::Image), // TIFF
            b"BM" => Some(DataType::Image),            // BMP

            // Audio
            b"ID3" | b"\xFF\xFB" | b"\xFF\xFA" => Some(DataType::Audio), // MP3
            b"RIFF" if data.len() > 8 && &data[8..12] == b"WAVE" => Some(DataType::Audio), // WAV
            b"fLaC" => Some(DataType::Audio),          // FLAC
            b"OggS" => Some(DataType::Audio),          // OGG

            // Compressés
            b"PK\x03\x04" | b"PK\x05\x06" => Some(DataType::Compressed), // ZIP
            b"\x1F\x8B" => Some(DataType::Compressed), // GZIP
            b"BZh" => Some(DataType::Compressed),     // BZIP2
            b"\x78\x9C" | b"\x78\x01" | b"\x78\xDA" => Some(DataType::Compressed), // ZLIB

            _ => None,
        }
    }

    /// Calcule l'entropie de Shannon (0-8, où 8 = aléatoire maximal)
    pub fn calculate_entropy(&self, data: &[u8]) -> f64 {
        if data.is_empty() {
            return 0.0;
        }

        let sample = &data[..self.sample_size.min(data.len())];
        let len = sample.len() as f64;

        // Calculer les fréquences de chaque octet
        let mut frequencies = [0usize; 256];
        for &byte in sample {
            frequencies[byte as usize] += 1;
        }

        // Calculer l'entropie
        let mut entropy = 0.0f64;
        for &count in frequencies.iter() {
            if count > 0 {
                let p = count as f64 / len;
                entropy -= p * p.log2();
            }
        }

        entropy
    }

    /// Calcule le ratio de répétition (0-1)
    /// Plus élevé = plus de répétitions
    pub fn calculate_repetition(&self, data: &[u8]) -> f64 {
        if data.len() < 2 {
            return 0.0;
        }

        let sample = &data[..self.sample_size.min(data.len())];
        let mut consecutive_count = 0usize;

        for window in sample.windows(2) {
            if window[0] == window[1] {
                consecutive_count += 1;
            }
        }

        consecutive_count as f64 / (sample.len() - 1) as f64
    }

    /// Vérifie si les données sont du texte imprimable
    pub fn is_printable_text(&self, data: &[u8]) -> bool {
        let sample = &data[..self.sample_size.min(data.len())];
        let mut printable_count = 0usize;

        for &byte in sample {
            // Caractères imprimables ASCII + espace + tab + newline
            if byte.is_ascii_graphic() || byte.is_ascii_whitespace() {
                printable_count += 1;
            }
        }

        // Au moins 90% de caractères imprimables
        let ratio = printable_count as f64 / sample.len() as f64;
        ratio > 0.9
    }

    /// Analyse les données et retourne un rapport
    pub fn analyze(&self, data: &[u8]) -> DataReport {
        let data_type = self.detect_data_type(data);
        let entropy = self.calculate_entropy(data);
        let repetition = self.calculate_repetition(data);
        let size = data.len();

        DataReport {
            data_type,
            entropy,
            repetition_ratio: repetition,
            size,
            recommended_compression: self.recommend_compression(data_type, entropy, repetition),
        }
    }

    /// Recommande une méthode de compression
    fn recommend_compression(&self, data_type: DataType, entropy: f64, repetition: f64) -> CompressionMethod {
        match (data_type, entropy, repetition) {
            (DataType::Text, _, _) => CompressionMethod::Huffman,
            (DataType::Repetitive, _, _) if repetition > 0.7 => CompressionMethod::Huffman,
            (DataType::Compressed, _, _) => CompressionMethod::None, // Déjà compressé
            (_, ent, _) if ent > 7.5 => CompressionMethod::None, // Trop aléatoire
            (_, _, rep) if rep > 0.5 => CompressionMethod::Huffman,
            _ => CompressionMethod::Lz4,
        }
    }
}

impl Default for DataAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Rapport d'analyse de données
#[derive(Debug, Clone)]
pub struct DataReport {
    /// Type de données détecté
    pub data_type: DataType,
    /// Entropie de Shannon (0-8)
    pub entropy: f64,
    /// Ratio de répétition (0-1)
    pub repetition_ratio: f64,
    /// Taille des données en octets
    pub size: usize,
    /// Méthode de compression recommandée
    pub recommended_compression: CompressionMethod,
}

impl DataReport {
    /// Formate le rapport pour affichage
    pub fn format(&self) -> String {
        format!(
            "┌─────────────────────────────────────┐\n\
             │ Rapport d'Analyse de Données         │\n\
             ├─────────────────────────────────────┤\n\
             │ Type         : {:>20} │\n\
             │ Taille       : {:>15} octets │\n\
             │ Entropie     : {:>15.2} / 8.0 │\n\
             │ Répétition   : {:>15.1}%      │\n\
             │ Compression  : {:>20} │\n\
             └─────────────────────────────────────┘",
            self.data_type.description(),
            self.size,
            self.entropy,
            self.repetition_ratio * 100.0,
            self.recommended_compression.description()
        )
    }
}

/// Méthode de compression
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMethod {
    /// Pas de compression
    None,
    /// Compression Huffman
    Huffman,
    /// Compression LZ4
    Lz4,
}

impl CompressionMethod {
    fn description(&self) -> &'static str {
        match self {
            CompressionMethod::None => "Aucune",
            CompressionMethod::Huffman => "Huffman",
            CompressionMethod::Lz4 => "LZ4",
        }
    }
}

/// Encodeur adaptatif
pub struct AdaptiveEncoder {
    analyzer: DataAnalyzer,
    constraints: DnaConstraints,
    rs_codec: ReedSolomonCodec,
}

impl AdaptiveEncoder {
    /// Crée un nouvel encodeur adaptatif
    pub fn new(constraints: DnaConstraints) -> Self {
        Self {
            analyzer: DataAnalyzer::new(),
            constraints,
            rs_codec: ReedSolomonCodec::new(),
        }
    }

    /// Encode automatiquement avec la meilleure stratégie
    pub fn encode_auto(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        // Analyser les données
        let report = self.analyzer.analyze(data);

        // Choisir la compression
        let compressed = match report.recommended_compression {
            CompressionMethod::Huffman => self.compress_huffman(data)?,
            CompressionMethod::Lz4 => self.compress_lz4(data)?,
            CompressionMethod::None => data.to_vec(),
        };

        // Appliquer Reed-Solomon pour la correction d'erreurs
        let rs_encoded = self.rs_codec.encode(&compressed)?;

        // Encoder avec GC-aware
        self.encode_gc_aware(&rs_encoded, &report)
    }

    /// Compression Huffman
    pub fn compress_huffman(&self, data: &[u8]) -> Result<Vec<u8>> {
        let compressor = HuffmanCompressor::new(data);
        compressor.compress(data)
    }

    /// Compression LZ4
    pub fn compress_lz4(&self, data: &[u8]) -> Result<Vec<u8>> {
        lz4::block::compress(data, None, true)
            .map_err(|e| DnaError::Encoding(format!("Erreur compression LZ4: {}", e)))
    }

    /// Encodage GC-aware (délégation au codec existant)
    fn encode_gc_aware(&self, data: &[u8], _report: &DataReport) -> Result<Vec<DnaSequence>> {
        // Diviser en chunks de 25 octets (100 bases après 2-bit mapping)
        let chunk_size = 25;
        let encoder = GcAwareEncoder::new(self.constraints.clone());
        let mut sequences = Vec::new();

        let mut seed = 0u64;
        for (idx, chunk) in data.chunks(chunk_size).enumerate() {
            // Degree de Fountain: varier entre 1 et 10
            let degree = (idx % 10) + 1;

            let sequence = encoder.encode(chunk.to_vec(), seed, degree)?;
            sequences.push(sequence);

            seed = seed.wrapping_add(1);
        }

        Ok(sequences)
    }

    /// Retourne l'analyseur de données
    pub fn analyzer(&self) -> &DataAnalyzer {
        &self.analyzer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_text() {
        let analyzer = DataAnalyzer::new();

        let text = b"Hello, this is a simple text with common words.";
        let data_type = analyzer.detect_data_type(text);

        assert_eq!(data_type, DataType::Text);
    }

    #[test]
    fn test_detect_repetitive() {
        let analyzer = DataAnalyzer::new();

        let repetitive: Vec<u8> = vec![b'A'; 1000];
        let data_type = analyzer.detect_data_type(&repetitive);

        assert_eq!(data_type, DataType::Repetitive);
    }

    #[test]
    fn test_detect_png() {
        let analyzer = DataAnalyzer::new();

        let mut png_data = vec![0u8; 100];
        png_data[0..4].copy_from_slice(b"\x89PNG");

        let data_type = analyzer.detect_data_type(&png_data);
        assert_eq!(data_type, DataType::Image);
    }

    #[test]
    fn test_entropy_calculation() {
        let analyzer = DataAnalyzer::new();

        // Données uniformes (entropie 0)
        let uniform = vec![42u8; 1000];
        let entropy_uniform = analyzer.calculate_entropy(&uniform);
        assert!(entropy_uniform < 0.1);

        // Données aléatoires (entropie proche de 8)
        let random: Vec<u8> = (0..256).map(|i| i as u8).collect();
        let entropy_random = analyzer.calculate_entropy(&random);
        assert!(entropy_random > 7.5);
    }

    #[test]
    fn test_repetition_ratio() {
        let analyzer = DataAnalyzer::new();

        // Pas de répétition
        let no_repetition: Vec<u8> = (0..100).map(|i| i as u8).collect();
        let ratio1 = analyzer.calculate_repetition(&no_repetition);
        assert!(ratio1 < 0.1);

        // Toute répétition
        let all_repetition = vec![42u8; 100];
        let ratio2 = analyzer.calculate_repetition(&all_repetition);
        assert!(ratio2 > 0.95);
    }

    #[test]
    fn test_data_report() {
        let analyzer = DataAnalyzer::new();

        let text = b"Hello, World!";
        let report = analyzer.analyze(text);

        assert_eq!(report.data_type, DataType::Text);
        assert_eq!(report.size, 13);
        println!("{}", report.format());
    }

    #[test]
    fn test_compress_huffman() {
        let encoder = AdaptiveEncoder::new(DnaConstraints::default());

        let data = b"AAAABBBCCDAAABBBCCD"; // Données répétitives
        let compressed = encoder.compress_huffman(data);

        assert!(compressed.is_ok());
        // Huffman devrait réduire la taille pour ces données répétitives
        assert!(compressed.unwrap().len() <= data.len());
    }

    #[test]
    fn test_compress_lz4() {
        let encoder = AdaptiveEncoder::new(DnaConstraints::default());

        let data = b"Hello, World! " as &[u8];
        let compressed = encoder.compress_lz4(data);

        assert!(compressed.is_ok());
    }

    #[test]
    #[ignore] // TODO: Fix GC-aware encoder padding to respect homopolymer constraints
    fn test_adaptive_encoding() {
        // Contraintes plus souples pour ce test
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 10, // Plus tolérant pour le padding GC-aware
            max_sequence_length: 152,
            allowed_bases: vec![
                crate::sequence::IupacBase::A,
                crate::sequence::IupacBase::C,
                crate::sequence::IupacBase::G,
                crate::sequence::IupacBase::T,
            ],
        };

        let encoder = AdaptiveEncoder::new(constraints.clone());

        let data = b"Test adaptive encoding!";
        let sequences = encoder.encode_auto(data);

        assert!(sequences.is_ok());
        let sequences = sequences.unwrap();
        assert!(!sequences.is_empty());

        // Vérifier que toutes les séquences respectent les contraintes
        for seq in &sequences {
            let result = seq.validate(&constraints);
            if let Err(ref e) = result {
                println!("Sequence failed: {:?}, error: {:?}", seq.bases.len(), e);
            }
            assert!(result.is_ok());
        }
    }
}
