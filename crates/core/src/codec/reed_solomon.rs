//! Codec Reed-Solomon pour la correction d'erreurs
//!
//! Utilise Reed-Solomon pour ajouter de la redondance aux données
//! permettant de corriger des erreurs de transmission/stockage
//!
//! Note: Cette implémentation chunk les données en blocs de 223 bytes
//! avec 32 bytes de ECC par bloc (standard Reed-Solomon 255, 223)

use crate::error::{DnaError, Result};
use reed_solomon::{Encoder, Decoder};

/// Taille max des données par bloc (255 total - 32 ECC)
const MAX_DATA_BLOCK_SIZE: usize = 223;

/// Codec Reed-Solomon pour la correction d'erreurs
///
/// Utilise Reed-Solomon (255, 223) standard:
/// - 223 bytes de données par bloc
/// - 32 bytes de ECC par bloc
/// - Peut corriger jusqu'à 16 erreurs ou 32 effacements connus par bloc
pub struct ReedSolomonCodec {
    encoder: Encoder,
    decoder: Decoder,
    ecc_len: usize,
    max_data_block: usize,
}

impl ReedSolomonCodec {
    /// Crée un nouveau codec Reed-Solomon avec 32 bytes de ECC
    /// (peut corriger jusqu'à 16 erreurs ou 32 effacements connus)
    pub fn new() -> Self {
        let ecc_len = 32;
        let encoder = Encoder::new(ecc_len);
        let decoder = Decoder::new(ecc_len);
        let max_data_block = MAX_DATA_BLOCK_SIZE;

        Self {
            encoder,
            decoder,
            ecc_len,
            max_data_block,
        }
    }

    /// Crée un codec avec des paramètres personnalisés
    ///
    /// # Arguments
    /// * `ecc_len` - Nombre d'bytes de ECC par bloc
    ///
    /// Note: La taille max des données par bloc sera (255 - ecc_len)
    /// pour s'assurer que block_size < 256 (requis par le décodeur)
    pub fn with_ecc_len(ecc_len: usize) -> Self {
        // S'assurer que block_size < 256 (requis par reed-solomon decoder)
        let max_data_block = 255 - ecc_len;
        let encoder = Encoder::new(ecc_len);
        let decoder = Decoder::new(ecc_len);

        Self {
            encoder,
            decoder,
            ecc_len,
            max_data_block,
        }
    }

    /// Encode les données avec Reed-Solomon ECC
    ///
    /// Les données sont divisées en blocs de max_data_block bytes,
    /// chaque bloc reçoit ecc_len bytes de ECC
    ///
    /// Format: [original_len (4 bytes)] [encoded blocks...]
    pub fn encode(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();

        // Stocker la longueur originale sur 4 bytes (big-endian)
        let original_len = (data.len() as u32).to_be_bytes();
        result.extend_from_slice(&original_len);

        // Chunk les données et encoder chaque bloc
        for chunk in data.chunks(self.max_data_block) {
            // Créer un bloc paddé à max_data_block
            let mut block = vec![0u8; self.max_data_block];
            block[..chunk.len()].copy_from_slice(chunk);

            let encoded = self.encoder.encode(&block);
            result.extend_from_slice(&encoded);
        }

        Ok(result)
    }

    /// Tente de décoder et corriger les données
    ///
    /// Décode chaque bloc et tente de corriger les erreurs
    pub fn decode(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let block_size = self.max_data_block + self.ecc_len;

        // Extraire la longueur originale (4 bytes)
        if data.len() < 4 {
            return Err(DnaError::Correction(
                "Données Reed-Solomon trop courtes (pas de longueur)".to_string()
            ));
        }

        let original_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let encoded_data = &data[4..];

        if encoded_data.len() % block_size != 0 {
            return Err(DnaError::Correction(format!(
                "Longueur des données invalide pour Reed-Solomon: {} (pas multiple de {})",
                encoded_data.len(),
                block_size
            )));
        }

        let mut result = Vec::new();

        // Décoder chaque bloc
        for block in encoded_data.chunks(block_size) {
            match self.decoder.correct(block, None) {
                Ok(corrected) => {
                    result.extend_from_slice(corrected.data());
                }
                Err(_) => {
                    return Err(DnaError::Correction(
                        format!("Reed-Solomon: correction impossible pour un bloc de {} bytes", block.len())
                    ));
                }
            }
        }

        // Tronquer à la longueur originale
        result.truncate(original_len);

        Ok(result)
    }

    /// Tente de décoder avec des positions d'effacements connues
    ///
    /// # Arguments
    /// * `data` - Les données encodées (data + ecc)
    /// * `erasure_positions` - Positions connues des erreurs (indices dans le buffer complet, après le préfixe de 4 bytes)
    pub fn decode_with_erasures(&self, data: &[u8], erasure_positions: &[usize]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let block_size = self.max_data_block + self.ecc_len;

        // Extraire la longueur originale (4 bytes)
        if data.len() < 4 {
            return Err(DnaError::Correction(
                "Données Reed-Solomon trop courtes".to_string()
            ));
        }

        let original_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let encoded_data = &data[4..];

        if encoded_data.len() % block_size != 0 {
            return Err(DnaError::Correction(
                "Longueur des données invalide pour Reed-Solomon".to_string()
            ));
        }

        let mut result = Vec::new();

        // Grouper les erasures par bloc
        for (block_idx, block) in encoded_data.chunks(block_size).enumerate() {
            let block_erasures: Vec<u8> = erasure_positions
                .iter()
                .filter(|&&pos| pos / block_size == block_idx)
                .map(|&pos| (pos % block_size) as u8)
                .collect();

            let positions = if block_erasures.is_empty() {
                None
            } else {
                Some(&block_erasures[..])
            };

            match self.decoder.correct(block, positions) {
                Ok(corrected) => {
                    result.extend_from_slice(corrected.data());
                }
                Err(_) => {
                    return Err(DnaError::Correction(
                        "Correction avec effacements impossible".to_string()
                    ));
                }
            }
        }

        // Tronquer à la longueur originale
        result.truncate(original_len);

        Ok(result)
    }

    /// Vérifie si les données contiennent des erreurs (sans correction)
    pub fn is_corrupted(&self, data: &[u8]) -> bool {
        if data.is_empty() {
            return false;
        }

        // Sauter le préfixe de 4 bytes
        let encoded_data = &data[4..];
        let block_size = self.max_data_block + self.ecc_len;

        encoded_data.chunks(block_size).any(|block| {
            self.decoder.is_corrupted(block)
        })
    }

    /// Retourne la longueur du ECC en bytes par bloc
    pub fn ecc_len(&self) -> usize {
        self.ecc_len
    }

    /// Retourne la taille max des données par bloc
    pub fn max_data_block(&self) -> usize {
        self.max_data_block
    }

    /// Retourne la taille totale d'un bloc (data + ecc)
    pub fn block_size(&self) -> usize {
        self.max_data_block + self.ecc_len
    }

    /// Retourne le nombre maximal d'erreurs corrigeables par bloc (sans positions connues)
    pub fn max_errors_per_block(&self) -> usize {
        self.ecc_len / 2
    }

    /// Retourne le nombre maximal d'effacements corrigeables par bloc (avec positions connues)
    pub fn max_erasures_per_block(&self) -> usize {
        self.ecc_len
    }

    /// Retourne le nombre de blocs pour des données de taille donnée
    pub fn num_blocks(&self, data_len: usize) -> usize {
        if data_len == 0 {
            0
        } else {
            (data_len + self.max_data_block - 1) / self.max_data_block
        }
    }

    /// Retourne la taille encodée pour des données de taille donnée
    /// (incluant le préfixe de 4 bytes pour la longueur)
    pub fn encoded_size(&self, data_len: usize) -> usize {
        4 + self.num_blocks(data_len) * self.block_size()
    }

    /// Retourne le pourcentage d'overhead (ECC / data)
    pub fn overhead_ratio(&self, data_len: usize) -> f64 {
        if data_len == 0 {
            0.0
        } else {
            let encoded = self.encoded_size(data_len);
            (encoded - data_len) as f64 / data_len as f64
        }
    }
}

impl Default for ReedSolomonCodec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reed_solomon_creation() {
        let codec = ReedSolomonCodec::new();
        assert_eq!(codec.ecc_len(), 32);
        assert_eq!(codec.max_data_block(), 223);
        assert_eq!(codec.block_size(), 255);
        assert_eq!(codec.max_errors_per_block(), 16);
        assert_eq!(codec.max_erasures_per_block(), 32);
    }

    #[test]
    fn test_encode_decode() {
        let codec = ReedSolomonCodec::new();
        let original = b"Hello, Reed-Solomon! This is a test.";

        let encoded = codec.encode(original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original.to_vec(), recovered);
        assert_eq!(encoded.len(), codec.encoded_size(original.len()));
    }

    #[test]
    fn test_error_correction() {
        let codec = ReedSolomonCodec::new();
        let original = b"Test data for error correction!";

        let mut encoded = codec.encode(original).unwrap();

        // Corrompre quelques bytes dans le premier bloc (après le préfixe de 4 bytes)
        encoded[9] = 0xFF;  // 5 + 4
        encoded[14] = 0xFF; // 10 + 4
        encoded[19] = 0xFF; // 15 + 4

        // Vérifier que les données sont corrompues
        assert!(codec.is_corrupted(&encoded));

        // Corriger
        let recovered = codec.decode(&encoded).unwrap();
        assert_eq!(original.to_vec(), recovered);
    }

    #[test]
    fn test_erasure_correction() {
        let codec = ReedSolomonCodec::new();
        let original = b"Testing erasure correction with known positions.";

        let mut encoded = codec.encode(original).unwrap();

        // Corrompre et noter les positions (offset de 4 bytes pour le préfixe de longueur)
        let erasure_positions = vec![9usize, 14, 19, 24]; // 5+4, 10+4, 15+4, 20+4
        for &pos in &erasure_positions {
            encoded[pos] = 0xFF;
        }

        // Corriger avec positions connues (les positions doivent être relatives aux données encodées, après le préfixe)
        let relative_positions: Vec<usize> = erasure_positions.iter().map(|&p| p - 4).collect();
        let recovered = codec.decode_with_erasures(&encoded, &relative_positions).unwrap();
        assert_eq!(original.to_vec(), recovered);
    }

    #[test]
    fn test_empty_data() {
        let codec = ReedSolomonCodec::new();
        let data = [];

        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert!(encoded.is_empty());
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_large_data() {
        let codec = ReedSolomonCodec::new();
        let original: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();

        let encoded = codec.encode(&original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original, recovered);

        // Vérifier la taille encodée
        let expected_blocks = (original.len() + 223 - 1) / 223; // 45 blocs
        let expected_size = 4 + expected_blocks * 255; // 4 + 11475 = 11479 bytes
        assert_eq!(encoded.len(), expected_size);
    }

    #[test]
    fn test_overhead_ratio() {
        let codec = ReedSolomonCodec::new();

        // Pour exactement 223 bytes (1 bloc + 4 bytes préfixe)
        // encoded_size = 4 + 255 = 259
        // overhead = (259 - 223) / 223 = 36 / 223
        let ratio = codec.overhead_ratio(223);
        let expected = 36.0 / 223.0; // (32 ECC + 4 prefix) / 223
        assert!((ratio - expected).abs() < 1e-9);

        // Pour 446 bytes (2 blocs + 4 bytes préfixe)
        // encoded_size = 4 + 255 * 2 = 514
        // overhead = (514 - 446) / 446 = 68 / 446
        let ratio2 = codec.overhead_ratio(446);
        let expected2 = 68.0 / 446.0; // (64 ECC + 4 prefix) / 446
        assert!((ratio2 - expected2).abs() < 1e-9);
    }

    #[test]
    fn test_with_custom_ecc_len() {
        let codec = ReedSolomonCodec::with_ecc_len(10);
        assert_eq!(codec.ecc_len(), 10);
        assert_eq!(codec.max_data_block(), 245); // 255 - 10
        assert_eq!(codec.max_errors_per_block(), 5);

        let original = b"Short test";
        let encoded = codec.encode(original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original.to_vec(), recovered);
    }

    #[test]
    fn test_too_many_errors() {
        let codec = ReedSolomonCodec::with_ecc_len(4); // Peut corriger max 2 erreurs
        let original = b"Test";

        let mut encoded = codec.encode(original).unwrap();

        // Corrompre plus que la capacité de correction
        // Corrompre les bytes après le préfixe de longueur (4 bytes)
        encoded[4] = 0xFF;
        encoded[5] = 0xFF;
        encoded[6] = 0xFF;
        encoded[7] = 0xFF;

        // Devrait échouer
        let result = codec.decode(&encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_block_size_alignment() {
        let codec = ReedSolomonCodec::new();
        let original = b"Testing block alignment";

        let encoded = codec.encode(original).unwrap();

        // La longueur encodée (sans le préfixe de 4 bytes) doit être un multiple de 255
        assert_eq!((encoded.len() - 4) % 255, 0);
    }

    #[test]
    fn test_max_data_block_boundary() {
        let codec = ReedSolomonCodec::new();

        // Exactement 223 bytes
        let data1: Vec<u8> = vec![1u8; 223];
        let encoded1 = codec.encode(&data1).unwrap();
        assert_eq!(encoded1.len(), 4 + 255); // 4 bytes préfixe + 1 bloc

        // 224 bytes (doit faire 2 blocs)
        let data2: Vec<u8> = vec![2u8; 224];
        let encoded2 = codec.encode(&data2).unwrap();
        assert_eq!(encoded2.len(), 4 + 255 * 2); // 4 bytes préfixe + 2 blocs
    }
}
