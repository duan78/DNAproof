//! Encodeurs et décodeurs ADN

pub mod encoder;
pub mod decoder;
pub mod reed_solomon;
pub mod goldman_2013;

pub use encoder::{Encoder, EncoderConfig, EncoderType};
pub use decoder::{Decoder, DecoderConfig};
pub use reed_solomon::ReedSolomonCodec;
pub use goldman_2013::{Goldman2013Encoder, Goldman2013Decoder};

use crate::error::Result;
use crate::sequence::DnaSequence;

/// Codec combiné encodeur/décodeur
pub struct Codec {
    encoder_config: EncoderConfig,
    decoder_config: DecoderConfig,
}

impl Codec {
    /// Crée un nouveau codec avec les configurations par défaut
    pub fn new() -> Self {
        Self {
            encoder_config: EncoderConfig::default(),
            decoder_config: DecoderConfig::default(),
        }
    }

    /// Encode des données en séquences ADN
    pub fn encode(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        let encoder = Encoder::new(self.encoder_config.clone())?;
        encoder.encode(data)
    }

    /// Décode des séquences ADN en données
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        let decoder = Decoder::new(self.decoder_config.clone());
        decoder.decode(sequences)
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_roundtrip() {
        let mut codec = Codec::new();
        // Use old Goldman encoder with lenient constraints for generic Codec test
        codec.encoder_config.encoder_type = crate::codec::encoder::EncoderType::Goldman;
        codec.encoder_config.constraints = crate::sequence::DnaConstraints {
            gc_min: 0.15,
            gc_max: 0.85,
            max_homopolymer: 20,  // Old Goldman doesn't use rotation, can create long runs
            max_sequence_length: 200,
            allowed_bases: vec![
                crate::sequence::IupacBase::A,
                crate::sequence::IupacBase::C,
                crate::sequence::IupacBase::G,
                crate::sequence::IupacBase::T,
            ],
        };

        let original = b"Hello, DNA world!";

        let sequences = codec.encode(original).unwrap();
        let recovered = codec.decode(&sequences).unwrap();

        assert_eq!(original.to_vec(), recovered);
    }
}
