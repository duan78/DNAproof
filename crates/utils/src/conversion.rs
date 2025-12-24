//! Conversions entre octets et ADN

use adn_core::{IupacBase, Result};

/// Convertisseur d'octets vers ADN
pub struct BytesToDna {
    /// Mode d'encodage
    encoding_mode: EncodingMode,
}

/// Mode d'encodage
#[derive(Debug, Clone, Copy)]
pub enum EncodingMode {
    /// 2 bits par base (4 bases = 1 octet)
    Standard,
    /// Encodage optimisé
    Optimized,
}

impl Default for BytesToDna {
    fn default() -> Self {
        Self {
            encoding_mode: EncodingMode::Standard,
        }
    }
}

impl BytesToDna {
    /// Crée un nouveau convertisseur
    pub fn new() -> Self {
        Self::default()
    }

    /// Définit le mode d'encodage
    pub fn with_mode(mut self, mode: EncodingMode) -> Self {
        self.encoding_mode = mode;
        self
    }

    /// Convertit des octets en bases ADN
    pub fn convert(&self, data: &[u8]) -> Vec<IupacBase> {
        match self.encoding_mode {
            EncodingMode::Standard => self.convert_standard(data),
            EncodingMode::Optimized => self.convert_optimized(data),
        }
    }

    /// Conversion standard (2 bits/base)
    fn convert_standard(&self, data: &[u8]) -> Vec<IupacBase> {
        let mut bases = Vec::with_capacity(data.len() * 4);

        for byte in data {
            let bits = [
                (byte >> 6) & 0b11,
                (byte >> 4) & 0b11,
                (byte >> 2) & 0b11,
                byte & 0b11,
            ];

            for two_bits in bits {
                let base = match two_bits {
                    0b00 => IupacBase::A,
                    0b01 => IupacBase::C,
                    0b10 => IupacBase::G,
                    0b11 => IupacBase::T,
                    _ => unreachable!(),
                };
                bases.push(base);
            }
        }

        bases
    }

    /// Conversion optimisée
    fn convert_optimized(&self, data: &[u8]) -> Vec<IupacBase> {
        // Pour l'instant, même chose que standard
        self.convert_standard(data)
    }
}

/// Convertisseur ADN vers octets
pub struct DnaToBytes {
    /// Mode de décodage
    decoding_mode: DecodingMode,
}

/// Mode de décodage
#[derive(Debug, Clone, Copy)]
pub enum DecodingMode {
    /// 2 bits par base (4 bases = 1 octet)
    Standard,
    /// Décodage optimisé
    Optimized,
}

impl Default for DnaToBytes {
    fn default() -> Self {
        Self {
            decoding_mode: DecodingMode::Standard,
        }
    }
}

impl DnaToBytes {
    /// Crée un nouveau convertisseur
    pub fn new() -> Self {
        Self::default()
    }

    /// Définit le mode de décodage
    pub fn with_mode(mut self, mode: DecodingMode) -> Self {
        self.decoding_mode = mode;
        self
    }

    /// Convertit des bases ADN en octets
    pub fn convert(&self, bases: &[IupacBase]) -> Result<Vec<u8>> {
        match self.decoding_mode {
            DecodingMode::Standard => self.convert_standard(bases),
            DecodingMode::Optimized => self.convert_optimized(bases),
        }
    }

    /// Conversion standard (2 bits/base)
    fn convert_standard(&self, bases: &[IupacBase]) -> Result<Vec<u8>> {
        let mut data = Vec::new();

        for chunk in bases.chunks(4) {
            if chunk.len() < 4 {
                break; // Ignorer les bases incomplètes
            }

            let mut byte = 0u8;

            for (i, base) in chunk.iter().enumerate() {
                let bits = match base {
                    IupacBase::A => 0b00,
                    IupacBase::C => 0b01,
                    IupacBase::G => 0b10,
                    IupacBase::T => 0b11,
                    _ => {
                        return Err(adn_core::DnaError::Decoding(format!(
                            "Base non-standard: {:?}",
                            base
                        )))
                    }
                };

                byte |= bits << (6 - 2 * i);
            }

            data.push(byte);
        }

        Ok(data)
    }

    /// Conversion optimisée
    fn convert_optimized(&self, bases: &[IupacBase]) -> Result<Vec<u8>> {
        // Pour l'instant, même chose que standard
        self.convert_standard(bases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_dna_roundtrip() {
        let converter_to_dna = BytesToDna::new();
        let converter_to_bytes = DnaToBytes::new();

        let original = vec![0b10101010, 0b01010101];

        let bases = converter_to_dna.convert(&original);
        let recovered = converter_to_bytes.convert(&bases).unwrap();

        assert_eq!(original, recovered);
    }

    #[test]
    fn test_specific_byte_conversion() {
        let converter = BytesToDna::new();

        // 0b10101010 -> G T G T
        let bases = converter.convert(&[0b10101010]);

        assert_eq!(bases.len(), 4);
        assert_eq!(bases[0], IupacBase::G);
        assert_eq!(bases[1], IupacBase::T);
    }
}
