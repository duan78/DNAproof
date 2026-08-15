//! Encodeur Ultime - Toutes les optimisations combinées
//!
//! Ce module combine toutes les optimisations de la Phase 1:
//! - Encodage adaptatif selon le type de données
//! - Code d'étalement pour protection burst errors
//! - Reed-Solomon pour correction d'erreurs
//! - GC-aware encoding avec padding optimal

use crate::codec::adaptive::{AdaptiveEncoder, CompressionMethod};
use crate::codec::enhanced_gc_aware::{EnhancedGcAwareDecoder, EnhancedGcAwareEncoder};
use crate::codec::enhanced_reed_solomon::EnhancedReedSolomonCodec;
use crate::codec::huffman::DnaHuffmanCompressor;
use crate::error::{DnaError, Result};
use crate::sequence::{DnaConstraints, DnaSequence};

/// Configuration de l'encodeur ultime
#[derive(Debug, Clone)]
pub struct UltimateEncoderConfig {
    /// Utiliser l'encodage adaptatif
    pub use_adaptive: bool,
    /// Utiliser le code d'étalement
    pub use_spreading: bool,
    /// Block size pour le code d'étalement
    pub spreading_block_size: usize,
    /// Utiliser le padding GC optimal
    pub use_optimal_padding: bool,
    /// Longueur max de padding
    pub max_padding: usize,
}

impl Default for UltimateEncoderConfig {
    fn default() -> Self {
        Self {
            use_adaptive: true,
            use_spreading: true,
            spreading_block_size: 32,
            use_optimal_padding: true,
            max_padding: 50,
        }
    }
}

/// Encodeur ultime avec toutes les optimisations
pub struct UltimateEncoder {
    constraints: DnaConstraints,
    config: UltimateEncoderConfig,
    rs_codec: EnhancedReedSolomonCodec,
    gc_aware_encoder: EnhancedGcAwareEncoder,
    adaptive_encoder: Option<AdaptiveEncoder>,
}

impl UltimateEncoder {
    /// Crée un nouvel encodeur ultime
    pub fn new(constraints: DnaConstraints, config: UltimateEncoderConfig) -> Self {
        // Configurer le codec Reed-Solomon avec spreading
        let rs_codec = EnhancedReedSolomonCodec::new()
            .with_spreading_block_size(config.spreading_block_size)
            .with_spreading(config.use_spreading);

        // Configurer l'encodeur GC-aware avec padding optimal
        let mut gc_aware_encoder = EnhancedGcAwareEncoder::new(constraints.clone());
        if config.use_optimal_padding {
            gc_aware_encoder = gc_aware_encoder.with_max_padding(config.max_padding);
        }

        // Configurer l'encodeur adaptatif
        let adaptive_encoder = if config.use_adaptive {
            Some(AdaptiveEncoder::new(constraints.clone()))
        } else {
            None
        };

        Self {
            constraints,
            config,
            rs_codec,
            gc_aware_encoder,
            adaptive_encoder,
        }
    }

    /// Encode des données en séquences ADN avec toutes les optimisations
    ///
    /// # Pipeline d'encodage
    /// 1. Analyse adaptative du type de données (si activé)
    /// 2. Compression adaptative (Huffman/LZ4/Aucune)
    /// 3. Reed-Solomon ECC
    /// 4. Code d'étalement (si activé)
    /// 5. Encodage GC-aware avec padding optimal
    ///
    /// Les séquences sont taggées `ultimate#<méthode>` où `<méthode>` ∈
    /// {huffman, lz4, none} : le décodeur lit ce suffixe pour choisir la
    /// décompression inverse.
    pub fn encode(&mut self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Choisir la compression (adaptative ou défaut)
        let (compressed, method) = self.compress_data(data)?;

        // 2. Appliquer Reed-Solomon + Spreading
        let encoded = self.rs_codec.encode(&compressed)?;

        // 3. Encoder en GC-aware et tagger avec la méthode de compression
        let mut sequences = self.encode_gc_aware(&encoded)?;
        let scheme = format!("ultimate#{}", method.scheme_suffix());
        for seq in &mut sequences {
            seq.metadata.encoding_scheme = scheme.clone();
        }

        Ok(sequences)
    }

    /// Compresse les données selon le type
    fn compress_data(&self, data: &[u8]) -> Result<(Vec<u8>, CompressionMethod)> {
        if let Some(adaptive) = &self.adaptive_encoder {
            // Utiliser l'encodage adaptatif
            let analyzer = adaptive.analyzer();
            let report = analyzer.analyze(data);

            match report.recommended_compression {
                CompressionMethod::Huffman => {
                    // Huffman auto-contenu (table embarquée) pour être
                    // décompressable au décodage sans les données originales
                    let compressor = DnaHuffmanCompressor::new(data);
                    Ok((compressor.compress(data)?, CompressionMethod::Huffman))
                }
                CompressionMethod::Lz4 => {
                    Ok((adaptive.compress_lz4(data)?, CompressionMethod::Lz4))
                }
                CompressionMethod::None => Ok((data.to_vec(), CompressionMethod::None)),
            }
        } else {
            // Compression par défaut (LZ4)
            lz4::block::compress(data, None, true)
                .map(|compressed| (compressed, CompressionMethod::Lz4))
                .map_err(|e| DnaError::Encoding(format!("Erreur compression LZ4: {}", e)))
        }
    }

    /// Encode en GC-aware avec padding optimal
    fn encode_gc_aware(&mut self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        // Diviser en chunks de 25 octets
        let chunk_size = 25;
        let mut sequences = Vec::new();

        let mut seed = 0u64;
        for (idx, chunk) in data.chunks(chunk_size).enumerate() {
            // Degree de Fountain: varier entre 1 et 10
            let degree = (idx % 10) + 1;

            let sequence = self.gc_aware_encoder.encode(chunk.to_vec(), seed, degree)?;

            sequences.push(sequence);
            seed = seed.wrapping_add(1);
        }

        Ok(sequences)
    }

    /// Retourne la configuration actuelle
    pub fn config(&self) -> &UltimateEncoderConfig {
        &self.config
    }

    /// Retourne les contraintes ADN utilisées
    pub fn constraints(&self) -> &DnaConstraints {
        &self.constraints
    }

    /// Analyse les données et retourne un rapport
    pub fn analyze_data(&self, data: &[u8]) -> Result<String> {
        if let Some(adaptive) = &self.adaptive_encoder {
            let analyzer = adaptive.analyzer();
            let report = analyzer.analyze(data);
            Ok(report.format())
        } else {
            Ok("Encodage adaptatif désactivé".to_string())
        }
    }
}

/// Décodeur ultime
pub struct UltimateDecoder {
    constraints: DnaConstraints,
    rs_codec: EnhancedReedSolomonCodec,
    gc_aware_decoder: EnhancedGcAwareDecoder,
}

impl UltimateDecoder {
    /// Crée un nouveau décodeur ultime avec les paramètres par défaut.
    ///
    /// Note : pour un round-trip correct, le décodeur doit avoir les mêmes
    /// paramètres de spreading que l'encodeur. Utiliser `with_config()`.
    pub fn new(constraints: DnaConstraints) -> Self {
        Self {
            constraints: constraints.clone(),
            rs_codec: EnhancedReedSolomonCodec::new(),
            gc_aware_decoder: EnhancedGcAwareDecoder::new(constraints),
        }
    }

    /// Crée un décodeur avec la même configuration que l'encodeur.
    ///
    /// C'est essentiel pour que le spreading block_size et l'activation
    /// du spreading correspondent entre encode et decode.
    pub fn with_config(constraints: DnaConstraints, config: &UltimateEncoderConfig) -> Self {
        let rs_codec = EnhancedReedSolomonCodec::new()
            .with_spreading_block_size(config.spreading_block_size)
            .with_spreading(config.use_spreading);
        Self {
            constraints: constraints.clone(),
            rs_codec,
            gc_aware_decoder: EnhancedGcAwareDecoder::new(constraints),
        }
    }

    /// Décode des séquences ADN en données
    ///
    /// # Pipeline de décodage
    /// 1. Décodage GC-aware
    /// 2. Reed-Solomon correction
    /// 3. Désentrelacement
    /// 4. Décompression selon la méthode taggée dans le schéma
    ///    (`ultimate#huffman` / `ultimate#lz4` / `ultimate#none`)
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        if sequences.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Décoder toutes les séquences GC-aware
        let mut chunks = Vec::new();
        for seq in sequences {
            let chunk = self.gc_aware_decoder.decode(seq)?;
            chunks.extend_from_slice(&chunk);
        }

        // 2. Décoder Reed-Solomon (avec désentrelacement intégré)
        let rs_decoded = self.rs_codec.decode(&chunks)?;

        // 3. Décompresser selon la méthode enregistrée par l'encodeur dans le
        // schéma de la première séquence. Sans suffixe (ancien format), LZ4.
        let scheme = sequences[0].metadata.encoding_scheme.as_str();
        let method = scheme.split_once('#').map(|(_, m)| m).unwrap_or("lz4");

        match method {
            "huffman" => DnaHuffmanCompressor::decompress(&rs_decoded),
            "none" => Ok(rs_decoded),
            "lz4" => lz4::block::decompress(&rs_decoded, None)
                .map_err(|e| DnaError::Decoding(format!("Erreur décompression LZ4: {}", e))),
            other => Err(DnaError::Decoding(format!(
                "Méthode de compression inconnue pour ultimate: {}",
                other
            ))),
        }
    }

    /// Retourne les contraintes ADN utilisées
    pub fn constraints(&self) -> &DnaConstraints {
        &self.constraints
    }
}

impl Default for UltimateDecoder {
    fn default() -> Self {
        Self::new(DnaConstraints::default())
    }
}

/// Codec ultime combiné
pub struct UltimateCodec {
    encoder: UltimateEncoder,
    decoder: UltimateDecoder,
}

impl UltimateCodec {
    /// Crée un nouveau codec ultime.
    ///
    /// Le décodeur est configuré avec les mêmes paramètres de spreading que
    /// l'encodeur pour garantir la cohérence du round-trip.
    pub fn new(constraints: DnaConstraints, config: UltimateEncoderConfig) -> Self {
        let encoder = UltimateEncoder::new(constraints.clone(), config.clone());
        let decoder = UltimateDecoder::with_config(constraints, &config);

        Self { encoder, decoder }
    }

    /// Encode des données
    pub fn encode(&mut self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        self.encoder.encode(data)
    }

    /// Décode des séquences
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        self.decoder.decode(sequences)
    }

    /// Analyse les données avant encodage
    pub fn analyze(&self, data: &[u8]) -> Result<String> {
        self.encoder.analyze_data(data)
    }

    /// Retourne l'encodeur
    pub fn encoder(&self) -> &UltimateEncoder {
        &self.encoder
    }

    /// Retourne le décodeur
    pub fn decoder(&self) -> &UltimateDecoder {
        &self.decoder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::IupacBase;

    #[test]
    fn test_ultimate_codec_roundtrip() {
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 10,
            max_sequence_length: 152,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let config = UltimateEncoderConfig {
            use_adaptive: false,
            use_spreading: true,
            spreading_block_size: 16,
            use_optimal_padding: true,
            max_padding: 30,
        };

        let mut codec = UltimateCodec::new(constraints, config);

        let original = b"Ultimate codec test!";
        let sequences = codec.encode(original).unwrap();

        assert!(!sequences.is_empty());

        // Round-trip strict : le décodeur est maintenant configuré avec les mêmes
        // paramètres de spreading que l'encodeur.
        let decoded = codec.decode(&sequences).unwrap();
        assert_eq!(
            original.to_vec(),
            decoded,
            "Ultimate round-trip must be exact"
        );
    }

    #[test]
    fn test_data_analysis() {
        let constraints = DnaConstraints::default();
        let config = UltimateEncoderConfig::default();

        let encoder = UltimateEncoder::new(constraints, config);

        let text_data = b"This is a test text for adaptive encoding!";
        let analysis = encoder.analyze_data(text_data);

        assert!(analysis.is_ok());
        println!("{}", analysis.unwrap());
    }

    #[test]
    fn test_config_default() {
        let config = UltimateEncoderConfig::default();

        assert!(config.use_adaptive);
        assert!(config.use_spreading);
        assert_eq!(config.spreading_block_size, 32);
        assert!(config.use_optimal_padding);
    }

    #[test]
    fn test_custom_config() {
        let config = UltimateEncoderConfig {
            use_adaptive: false,
            use_spreading: false,
            spreading_block_size: 16,
            use_optimal_padding: false,
            max_padding: 20,
        };

        let constraints = DnaConstraints::default();
        let encoder = UltimateEncoder::new(constraints, config);

        assert!(!encoder.config().use_adaptive);
        assert!(!encoder.config().use_spreading);
        assert!(!encoder.config().use_optimal_padding);
    }
}
