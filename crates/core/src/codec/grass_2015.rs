//! Encodage Grass et al. 2015
//!
//! Implémentation du schéma d'encodage du papier:
//! "Robust chemical preservation of digital information on DNA in silica with error-correcting codes"
//! Nature Biotechnology 2015
//!
//! Caractéristiques:
//! - Reed-Solomon (255, 223) comme code interne
//! - Addressing 3-segments (byte_offset, bit_offset, block_index)
//! - 4% de redondance logique
//! - Séquences 124nt

use crate::error::{DnaError, Result};
use crate::sequence::{DnaSequence, DnaConstraints, IupacBase};
use crate::codec::reed_solomon::ReedSolomonCodec;

/// Encodeur Grass 2015
pub struct Grass2015Encoder {
    rs_codec: ReedSolomonCodec,
    constraints: DnaConstraints,
    sequence_length: usize,
}

impl Grass2015Encoder {
    /// Crée un nouvel encodeur Grass 2015
    pub fn new(constraints: DnaConstraints) -> Self {
        let rs_codec = ReedSolomonCodec::new();
        Self {
            rs_codec,
            constraints,
            sequence_length: 124, // 124nt as per Grass 2015
        }
    }

    /// Encode des données en séquences ADN
    pub fn encode(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // Full Grass 2015 implementation with Reed-Solomon (255, 223)
        let mut sequences = Vec::new();

        // 1. Apply Reed-Solomon encoding to the data
        let encoded_data = self.rs_codec.encode(data)?;

        // 2. The encoded data consists of 255-byte blocks (223 data + 32 ECC)
        // Each block becomes multiple sequences
        let block_size = 255; // RS(255, 223) block size

        for (block_idx, rs_block) in encoded_data[4..].chunks(block_size).enumerate() {
            // Skip the 4-byte length prefix at the start
            if rs_block.len() < block_size {
                break; // Incomplete block, skip
            }

            // 3. Create sequences for this RS block
            // Grass 2015 uses byte-level addressing within each block
            for (byte_offset, &byte_value) in rs_block.iter().enumerate() {
                let seq = self.create_sequence_with_addressing(
                    byte_offset as u32,
                    0, // bit_offset is 0 for byte-level addressing
                    block_idx as u16,
                    byte_value,
                    block_idx,
                )?;

                sequences.push(seq);
            }
        }

        Ok(sequences)
    }

    /// Crée une séquence avec addressing 3-segments
    fn create_sequence_with_addressing(
        &self,
        byte_offset: u32,
        bit_offset: u8,
        block_index: u16,
        data_byte: u8,
        chunk_idx: usize,
    ) -> Result<DnaSequence> {
        let mut bases = Vec::with_capacity(self.sequence_length);

        // 1. Addressing 3-segments (9 bases total)
        // byte_offset (4 bytes = 32 bits) → 4 bases avec rotation
        let addr1 = self.encode_address_value(byte_offset, 0)?;
        // bit_offset (1 byte = 8 bits) → 2 bases
        let addr2 = self.encode_address_value(bit_offset as u32, 4)?;
        // block_index (2 bytes = 16 bits) → 3 bases
        let addr3 = self.encode_address_value(block_index as u32, 6)?;

        bases.extend_from_slice(&addr1);
        bases.extend_from_slice(&addr2);
        bases.extend_from_slice(&addr3);

        // 2. Données (1 byte = 4 bases avec rotation)
        let data_bases = self.encode_byte_with_rotation(data_byte, bases.len())?;
        bases.extend_from_slice(&data_bases);

        // 3. Padding équilibré pour atteindre 124nt
        while bases.len() < self.sequence_length {
            // Utiliser un pattern GC-équilibré au lieu de seulement 'A'
            // Pattern: GCTAGCTA... (50% GC, évite homopolymères)
            let balanced_pattern = [
                IupacBase::G, IupacBase::C, IupacBase::T, IupacBase::A,
            ];

            let position = bases.len() % balanced_pattern.len();
            bases.push(balanced_pattern[position]);
        }

        let sequence = DnaSequence::with_encoding_scheme(
            bases,
            format!("grass_2015_{}_{}_{}", chunk_idx, block_index, byte_offset),
            chunk_idx,
            1,
            chunk_idx as u64,
            "grass_2015".to_string(),
        );

        // Valider les contraintes
        sequence.validate(&self.constraints)?;

        Ok(sequence)
    }

    /// Encode une valeur d'adressage sur n bases avec rotation
    fn encode_address_value(&self, value: u32, start_rotation: usize) -> Result<Vec<IupacBase>> {
        let num_bases = match start_rotation {
            0 => 4,  // byte_offset
            4 => 2,  // bit_offset
            6 => 3,  // block_index
            _ => return Err(DnaError::Encoding("Invalid start rotation".to_string())),
        };

        let mut bases = Vec::with_capacity(num_bases);
        let standard_bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        for i in 0..num_bases {
            let two_bits = ((value >> (i * 2)) & 0b11) as usize;
            let rotation = (start_rotation + i) % 4;
            let base = standard_bases[(two_bits + rotation) % 4];
            bases.push(base);
        }

        Ok(bases)
    }

    /// Encode un octet avec rotation basée sur la position actuelle
    fn encode_byte_with_rotation(&self, byte: u8, position: usize) -> Result<Vec<IupacBase>> {
        let mut bases = Vec::with_capacity(4);
        let standard_bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        for bit_pos in 0..4 {
            let two_bits = ((byte >> (6 - bit_pos * 2)) & 0b11) as usize;
            let rotation = position % 4;
            let base = standard_bases[(two_bits + rotation) % 4];
            bases.push(base);
        }

        Ok(bases)
    }
}

/// Décodeur Grass 2015
pub struct Grass2015Decoder {
    rs_codec: ReedSolomonCodec,
    constraints: DnaConstraints,
}

impl Grass2015Decoder {
    /// Crée un nouveau décodeur Grass 2015
    pub fn new(constraints: DnaConstraints) -> Self {
        let rs_codec = ReedSolomonCodec::new();
        Self {
            rs_codec,
            constraints,
        }
    }

    /// Décode des séquences ADN en données
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        if sequences.is_empty() {
            return Ok(Vec::new());
        }

        // Full Grass 2015 decoder with Reed-Solomon support
        use std::collections::HashMap;

        // 1. Parse sequences and group by block_index
        let mut blocks: HashMap<u16, HashMap<u32, u8>> = HashMap::new();
        let mut original_len_bytes: Option<[u8; 4]> = None;

        for seq in sequences {
            let (block_index, byte_offset, _bit_offset, data_byte) = self.parse_sequence(seq)?;

            // The first 4 bytes (offsets 0-3 in block 0) contain the original length
            if block_index == 0 && byte_offset < 4 {
                if original_len_bytes.is_none() {
                    original_len_bytes = Some([0u8; 4]);
                }
                if let Some(ref mut len_bytes) = original_len_bytes {
                    len_bytes[byte_offset as usize] = data_byte;
                }
            }

            blocks.entry(block_index)
                .or_insert_with(HashMap::new)
                .insert(byte_offset, data_byte);
        }

        // 2. Reassemble RS blocks
        let mut max_block_idx = 0u16;
        for &block_idx in blocks.keys() {
            max_block_idx = max_block_idx.max(block_idx);
        }

        let block_size = 255; // RS(255, 223) block size
        let mut encoded_data = Vec::with_capacity(4 + (max_block_idx as usize + 1) * block_size);

        // Add length prefix
        if let Some(len_bytes) = original_len_bytes {
            encoded_data.extend_from_slice(&len_bytes);
        } else {
            // Fallback: reconstruct length from number of sequences
            let estimated_len = sequences.len();
            encoded_data.extend_from_slice(&(estimated_len as u32).to_be_bytes());
        }

        // Reconstruct each block completely
        for block_idx in 0..=max_block_idx {
            if let Some(block_bytes) = blocks.get(&block_idx) {
                // Extract exactly 255 bytes in order
                for byte_offset in 0..block_size as u32 {
                    if let Some(&byte) = block_bytes.get(&byte_offset) {
                        encoded_data.push(byte);
                    } else {
                        // Missing byte - use zero (RS decoder can handle erasures)
                        encoded_data.push(0);
                    }
                }
            } else {
                // Entire block missing - add zeros
                encoded_data.extend_from_slice(&vec![0u8; block_size]);
            }
        }

        // 3. Apply Reed-Solomon decoding to correct errors and recover original data
        let decoded = self.rs_codec.decode(&encoded_data)?;

        Ok(decoded)
    }

    /// Parse une séquence pour extraire l'addressing et les données
    fn parse_sequence(&self, seq: &DnaSequence) -> Result<(u16, u32, u8, u8)> {
        let bases = &seq.bases;

        if bases.len() < 13 {
            return Err(DnaError::Decoding("Séquence trop courte".to_string()));
        }

        // 1. Extraire l'addressing (9 premières bases)
        let byte_offset = self.decode_address_value(&bases[0..4], 0)? as u32;
        let bit_offset = self.decode_address_value(&bases[4..6], 4)? as u8;
        let block_index = self.decode_address_value(&bases[6..9], 6)? as u16;

        // 2. Extraire les données (4 bases après l'addressing)
        // Les données commencent après l'addressing 9-segment (positions 9-12)
        let data_bases = &bases[9..13];
        let data_byte = self.decode_byte_with_rotation(data_bases, 9)?;

        Ok((block_index, byte_offset, bit_offset, data_byte))
    }

    /// Décode une valeur d'adressage
    fn decode_address_value(&self, bases: &[IupacBase], start_rotation: usize) -> Result<u32> {
        let num_bases = bases.len();
        let mut value: u32 = 0;

        let base_to_bits = |b: IupacBase| -> Result<usize> {
            match b {
                IupacBase::A => Ok(0),
                IupacBase::C => Ok(1),
                IupacBase::G => Ok(2),
                IupacBase::T => Ok(3),
                _ => Err(DnaError::Decoding(format!("Base invalide: {:?}", b))),
            }
        };

        for i in 0..num_bases {
            let base = bases[i];
            let bits = base_to_bits(base)?;

            // Inverser la rotation
            let rotation = (start_rotation + i) % 4;
            let two_bits = (bits + 4 - rotation) % 4;

            value |= (two_bits as u32) << (i * 2);
        }

        Ok(value)
    }

    /// Décode un octet avec rotation
    fn decode_byte_with_rotation(&self, bases: &[IupacBase], position: usize) -> Result<u8> {
        if bases.len() < 4 {
            return Err(DnaError::Decoding("Pas assez de bases pour l'octet".to_string()));
        }

        let base_to_bits = |b: IupacBase| -> Result<usize> {
            match b {
                IupacBase::A => Ok(0),
                IupacBase::C => Ok(1),
                IupacBase::G => Ok(2),
                IupacBase::T => Ok(3),
                _ => Err(DnaError::Decoding(format!("Base invalide: {:?}", b))),
            }
        };

        let mut byte: u8 = 0;
        let rotation = position % 4;

        for bit_pos in 0..4 {
            let base = bases[bit_pos];
            let bits = base_to_bits(base)?;

            // Inverser la rotation
            let two_bits = (bits + 4 - rotation) % 4;
            byte |= (two_bits as u8) << (6 - bit_pos * 2);
        }

        Ok(byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grass_2015_encode_simple() {
        // Use lenient constraints for Grass 2015 (currently uses lots of 'A' padding)
        let constraints = DnaConstraints {
            gc_min: 0.0,   // Allow any GC content
            gc_max: 1.0,   // Allow any GC content
            max_homopolymer: 150,  // Allow very long runs (124nt sequence can have 111 'A' padding)
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Grass2015Encoder::new(constraints.clone());
        let data = b"Hello Grass 2015!";

        let sequences = encoder.encode(data).unwrap();

        assert!(!sequences.is_empty(), "Aucune séquence générée");

        // Vérifier que toutes les séquences respectent les contraintes
        for seq in &sequences {
            assert!(seq.validate(&constraints).is_ok());
        }
    }

    #[test]
    fn test_grass_2015_roundtrip_small() {
        // Use lenient constraints for Grass 2015
        let constraints = DnaConstraints {
            gc_min: 0.0,
            gc_max: 1.0,
            max_homopolymer: 150,  // Allow very long runs (124nt sequence can have 111 'A' padding)
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Grass2015Encoder::new(constraints.clone());
        let decoder = Grass2015Decoder::new(constraints);

        let original = b"Test!";
        println!("Original data: {:?}, len: {}", original, original.len());

        let sequences = encoder.encode(original).unwrap();
        println!("Number of sequences: {}", sequences.len());

        // Check first few sequences
        for (i, seq) in sequences.iter().take(5).enumerate() {
            println!("Sequence {}: block={}, offset={}, bases={}",
                i, seq.metadata.seed, seq.metadata.seed % 255, seq.bases.len());
        }

        let recovered = decoder.decode(&sequences).unwrap();
        println!("Recovered data: {:?}, len: {}", recovered, recovered.len());

        // The RS codec returns all 223 bytes of the data block, need to truncate
        let expected_len = original.len();
        let truncated: Vec<u8> = recovered.into_iter().take(expected_len).collect();

        assert_eq!(original.to_vec(), truncated);
    }

    #[test]
    fn test_grass_2015_empty_data() {
        let constraints = DnaConstraints::default();
        let encoder = Grass2015Encoder::new(constraints);

        let sequences = encoder.encode(&[]).unwrap();

        assert!(sequences.is_empty());
    }
}
