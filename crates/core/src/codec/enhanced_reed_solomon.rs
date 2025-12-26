//! Codec Reed-Solomon amélioré avec Code d'Étalement
//!
//! Ce module étend Reed-Solomon avec un code d'étalement pour protéger
//! contre les burst errors courants dans le séquençage ADN.

use crate::error::{DnaError, Result};
use crate::codec::reed_solomon::ReedSolomonCodec;
use crate::codec::spreading::SpreadingCode;

/// Codec Reed-Solomon amélioré avec code d'étalement
pub struct EnhancedReedSolomonCodec {
    /// Codec Reed-Solomon interne
    rs_codec: ReedSolomonCodec,
    /// Code d'étalement pour protection burst errors
    spreading: SpreadingCode,
    /// Utiliser le code d'étalement
    use_spreading: bool,
}

impl EnhancedReedSolomonCodec {
    /// Crée un nouveau codec avec code d'étalement activé
    pub fn new() -> Self {
        Self {
            rs_codec: ReedSolomonCodec::new(),
            spreading: SpreadingCode::default(), // block_size = 32
            use_spreading: true,
        }
    }

    /// Crée un codec sans code d'étalement
    pub fn without_spreading() -> Self {
        Self {
            rs_codec: ReedSolomonCodec::new(),
            spreading: SpreadingCode::default(),
            use_spreading: false,
        }
    }

    /// Configure le block_size du code d'étalement
    pub fn with_spreading_block_size(mut self, block_size: usize) -> Self {
        self.spreading = SpreadingCode::new(block_size);
        self
    }

    /// Active ou désactive le code d'étalement
    pub fn with_spreading(mut self, enabled: bool) -> Self {
        self.use_spreading = enabled;
        self
    }

    /// Encode les données avec Reed-Solomon + Spreading
    ///
    /// # Pipeline
    /// 1. Spreading code (si activé) - distribue les burst errors
    /// 2. Reed-Solomon ECC - corrige les erreurs
    pub fn encode(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Appliquer le code d'étalement si activé
        let spreaded = if self.use_spreading {
            self.spreading.interleave(data)
        } else {
            data.to_vec()
        };

        // 2. Appliquer Reed-Solomon
        let encoded = self.rs_codec.encode(&spreaded)?;

        Ok(encoded)
    }

    /// Décode les données
    ///
    /// # Pipeline
    /// 1. Reed-Solomon - corrige les erreurs
    /// 2. Désentrelacement (si activé) - reconstruit l'ordre original
    pub fn decode(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Décoder Reed-Solomon
        let decoded = self.rs_codec.decode(data)?;

        // 2. Désentrelacer si le spreading était activé
        let result = if self.use_spreading {
            self.spreading.deinterleave(&decoded)
        } else {
            decoded
        };

        Ok(result)
    }

    /// Décode avec positions d'effacements connus
    pub fn decode_with_erasures(&self, data: &[u8], erasure_positions: &[usize]) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Décoder Reed-Solomon avec effacements
        let decoded = self.rs_codec.decode_with_erasures(data, erasure_positions)?;

        // 2. Désentrelacer si nécessaire
        let result = if self.use_spreading {
            self.spreading.deinterleave(&decoded)
        } else {
            decoded
        };

        Ok(result)
    }

    /// Vérifie si les données contiennent des erreurs
    pub fn is_corrupted(&self, data: &[u8]) -> bool {
        self.rs_codec.is_corrupted(data)
    }

    /// Retourne la longueur du ECC en bytes par bloc
    pub fn ecc_len(&self) -> usize {
        self.rs_codec.ecc_len()
    }

    /// Retourne la taille max des données par bloc
    pub fn max_data_block(&self) -> usize {
        self.rs_codec.max_data_block()
    }

    /// Retourne la taille totale d'un bloc (data + ecc)
    pub fn block_size(&self) -> usize {
        self.rs_codec.block_size()
    }

    /// Retourne le nombre maximal d'erreurs corrigeables par bloc
    pub fn max_errors_per_block(&self) -> usize {
        self.rs_codec.max_errors_per_block()
    }

    /// Retourne si le code d'étalement est activé
    pub fn is_spreading_enabled(&self) -> bool {
        self.use_spreading
    }

    /// Retourne le block_size du code d'étalement
    pub fn spreading_block_size(&self) -> usize {
        self.spreading.block_size()
    }

    /// Retourne la protection contre les burst errors
    pub fn max_burst_protection(&self) -> usize {
        if self.use_spreading {
            self.spreading.max_burst_protection()
        } else {
            0
        }
    }
}

impl Default for EnhancedReedSolomonCodec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_rs_roundtrip() {
        let codec = EnhancedReedSolomonCodec::new();

        let original = b"Hello, Enhanced Reed-Solomon with Spreading!";
        let encoded = codec.encode(original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original.to_vec(), recovered);
    }

    #[test]
    fn test_without_spreading() {
        let codec = EnhancedReedSolomonCodec::without_spreading();

        let original = b"Test without spreading code";
        let encoded = codec.encode(original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original.to_vec(), recovered);
        assert!(!codec.is_spreading_enabled());
    }

    #[test]
    fn test_spreading_enabled() {
        let codec = EnhancedReedSolomonCodec::new();

        assert!(codec.is_spreading_enabled());
        assert_eq!(codec.spreading_block_size(), 32);
        assert_eq!(codec.max_burst_protection(), 32);
    }

    #[test]
    fn test_error_correction_with_spreading() {
        let codec = EnhancedReedSolomonCodec::new();

        let original = b"Testing error correction with burst error protection!";
        let mut encoded = codec.encode(original).unwrap();

        // Corrompre quelques bytes (après le préfixe de longueur)
        encoded[10] = 0xFF;
        encoded[11] = 0xFF;
        encoded[12] = 0xFF;

        // Vérifier que les données sont corrompues
        assert!(codec.is_corrupted(&encoded));

        // Corriger
        let recovered = codec.decode(&encoded).unwrap();
        assert_eq!(original.to_vec(), recovered);
    }

    #[test]
    fn test_custom_block_size() {
        let codec = EnhancedReedSolomonCodec::new()
            .with_spreading_block_size(16);

        assert_eq!(codec.spreading_block_size(), 16);
        assert_eq!(codec.max_burst_protection(), 16);
    }

    #[test]
    fn test_empty_data() {
        let codec = EnhancedReedSolomonCodec::new();

        let data = [];
        let encoded = codec.encode(&data).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        assert!(encoded.is_empty());
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_large_data() {
        let codec = EnhancedReedSolomonCodec::new();

        let original: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();

        let encoded = codec.encode(&original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original, recovered);
    }

    #[test]
    fn test_toggle_spreading() {
        let codec = EnhancedReedSolomonCodec::new()
            .with_spreading(false);

        assert!(!codec.is_spreading_enabled());

        let original = b"Test spreading toggle";
        let encoded = codec.encode(original).unwrap();
        let recovered = codec.decode(&encoded).unwrap();

        assert_eq!(original.to_vec(), recovered);
    }
}
