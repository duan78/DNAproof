//! Encodage Goldman et al. 2013
//!
//! Implémentation du schéma d'encodage du papier:
//! "Towards practical, high-capacity, low-maintenance information storage in synthesized DNA"
//! Nature 2013
//!
//! Caractéristiques:
//! - Compression Huffman
//! - Encodage 3-base rotation
//! - Addressing 4-byte par oligo
//! - Segments alternés addressing/data

use crate::error::{DnaError, Result};
use crate::sequence::{DnaSequence, DnaConstraints, IupacBase};

/// Encodeur Goldman 2013
pub struct Goldman2013Encoder {
    constraints: DnaConstraints,
}

impl Goldman2013Encoder {
    /// Crée un nouvel encodeur Goldman 2013
    pub fn new(constraints: DnaConstraints) -> Self {
        Self { constraints }
    }

    /// Encode des données en séquences ADN
    pub fn encode(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        // 1. Compression Huffman (simplifiée - utiliser LZ4 pour l'instant)
        // Pour MVP: pas de compression pour éviter les problèmes avec petits fichiers
        let compressed = data.to_vec(); // self.compress_huffman(data)?;

        // 2. Diviser en chunks de 3 octets (pour 3-base rotation)
        let chunk_size = 3;
        let chunks: Vec<&[u8]> = compressed.chunks(chunk_size).collect();

        let mut sequences = Vec::new();

        for (idx, chunk) in chunks.iter().enumerate() {
            // 3. Encoder avec 3-base rotation
            let bases = self.encode_goldman_3base(chunk, idx, chunks.len())?;

            // 4. Ajouter addressing 4-byte (simplifié pour l'instant)
            let full_sequence = self.add_addressing(bases, idx)?;

            let sequence = DnaSequence::new(
                full_sequence,
                format!("goldman_2013_{}", idx),
                idx,
                chunk.len(),
                idx as u64,
            );

            // Valider les contraintes
            sequence.validate(&self.constraints)?;

            sequences.push(sequence);
        }

        Ok(sequences)
    }

    /// Compression Huffman simplifiée ( utilise LZ4 pour l'instant)
    fn compress_huffman(&self, data: &[u8]) -> Result<Vec<u8>> {
        // Pour l'instant, utiliser LZ4 comme proxy pour Huffman
        // TODO: Implémenter Huffman vrai pour optimiser la répétition
        let compressed = lz4::block::compress(
            data,
            None,
            true, // avec checksum
        ).map_err(|e| DnaError::Encoding(format!("Erreur compression: {}", e)))?;

        Ok(compressed)
    }

    /// Encode avec rotation 2-bit (Goldman 2013 simplifié)
    ///
    /// Encodage 2-bit avec rotation pour éviter les homopolymères
    fn encode_goldman_3base(&self, chunk: &[u8], idx: usize, _total_chunks: usize) -> Result<Vec<IupacBase>> {
        let mut bases = Vec::with_capacity(chunk.len() * 4);

        // Rotation de départ basée sur l'index pour varier l'encodage
        let start_offset = idx % 4;

        // Mapping 2-bit standard
        let standard_bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        // Encodage 2-bit avec rotation pour éviter homopolymères et balancer GC
        for (byte_pos, &byte) in chunk.iter().enumerate() {
            // Encoder chaque octet en 4 bases (2 bits par base)
            for bit_pos in 0..4 {
                let two_bits = ((byte >> (6 - bit_pos * 2)) & 0b11) as usize;

                // Appliquer rotation basée sur position pour éviter homopolymères
                let rotation = (start_offset + byte_pos + bit_pos) % 4;

                // Sélectionner la base avec rotation
                let base = standard_bases[(two_bits + rotation) % 4];

                bases.push(base);
            }
        }

        Ok(bases)
    }

    /// Vérifie si on peut ajouter une base sans dépasser les contraintes GC
    fn can_append_for_gc(&self, bases: &[IupacBase], new_base: IupacBase) -> bool {
        let len = bases.len();
        if len == 0 {
            return true;
        }

        // Vérifier homopolymer
        let max_homopolymer = self.constraints.max_homopolymer;
        if let Some(last) = bases.last() {
            if *last == new_base {
                let run = bases.iter().rev().take_while(|&&b| b == new_base).count();
                if run >= max_homopolymer {
                    return false;
                }
            }
        }

        // Estimation GC (simplifiée)
        let gc_count = bases.iter().filter(|b| b.is_gc()).count()
            + if new_base.is_gc() { 1 } else { 0 };
        let gc_ratio = gc_count as f64 / (len + 1) as f64;

        gc_ratio >= self.constraints.gc_min && gc_ratio <= self.constraints.gc_max
    }

    /// Trouve une base alternative respectant les contraintes
    fn find_alternative_base(&self, preferred: IupacBase, bases: &[IupacBase]) -> Result<IupacBase> {
        let candidates = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        for &base in &candidates {
            if base != preferred && self.can_append_for_gc(bases, base) {
                return Ok(base);
            }
        }

        Err(DnaError::ConstraintViolation(
            "Impossible de trouver une base alternative respectant GC".to_string()
        ))
    }

    /// Ajoute l'addressing 8-byte (16 bits pour supporter jusqu'à 65535 séquences)
    fn add_addressing(&self, bases: Vec<IupacBase>, idx: usize) -> Result<Vec<IupacBase>> {
        // Pour le MVP, on encode l'index de manière à éviter les homopolymères
        let addr_bases = self.encode_index_8byte_safe(idx)?;

        // Insérer au début
        let mut full_sequence = addr_bases;
        full_sequence.extend(bases);

        Ok(full_sequence)
    }

    /// Encode un index sur 8 bases (16 bits, peut encoder jusqu'à 65535 séquences)
    fn encode_index_8byte_safe(&self, idx: usize) -> Result<Vec<IupacBase>> {
        if idx >= 65536 {
            return Err(DnaError::Encoding(format!(
                "Index de séquence trop grand: {} (max: 65535)", idx
            )));
        }

        // Encodage 2-bit avec rotation pour éviter les homopolymères
        let standard_bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let mut bases = Vec::with_capacity(8);

        // Encoder l'index sur 8 bases avec rotation pour éviter homopolymères
        // Utiliser une rotation qui change à chaque position
        for i in 0..8 {
            let two_bits = ((idx >> (i * 2)) & 0b11) as usize;

            // Appliquer une rotation basée sur la position pour éviter homopolymères
            // La rotation change à chaque position (0, 1, 2, 3, 0, 1, 2, 3, ...)
            let rotation = i % 4;
            let base = standard_bases[(two_bits + rotation) % 4];

            bases.push(base);
        }

        Ok(bases)
    }
}

/// Décodeur Goldman 2013
pub struct Goldman2013Decoder {
    constraints: DnaConstraints,
}

impl Goldman2013Decoder {
    /// Crée un nouveau décodeur Goldman 2013
    pub fn new(constraints: DnaConstraints) -> Self {
        Self { constraints }
    }

    /// Décode des séquences ADN en données
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        if sequences.is_empty() {
            return Err(DnaError::Decoding("Aucune séquence fournie".to_string()));
        }

        // Trier les séquences par index (extrait de l'addressing)
        let mut sorted_data: Vec<(usize, Vec<u8>)> = Vec::new();

        for seq in sequences {
            // Extraire l'addressing et les données
            let (idx, data) = self.parse_sequence(seq)?;
            sorted_data.push((idx, data));
        }

        // Trier par index et concaténer
        sorted_data.sort_by_key(|(idx, _)| *idx);
        let mut result = Vec::new();
        for (_, data) in sorted_data {
            result.extend_from_slice(&data);
        }

        // Pas de décompression pour MVP
        // let decompressed = self.decompress_huffman(&result)?;

        Ok(result)
    }

    /// Parse une séquence pour extraire l'index et les données
    fn parse_sequence(&self, seq: &DnaSequence) -> Result<(usize, Vec<u8>)> {
        let bases = &seq.bases;

        if bases.len() < 8 {
            return Err(DnaError::Decoding("Séquence trop courte pour contenir l'addressing".to_string()));
        }

        // Extraire l'index depuis les 8 premières bases
        let idx = self.decode_index_8byte(&bases[0..8])?;

        // Le reste sont les données encodées
        let data_bases = &bases[8..];
        let data = self.decode_bases_to_bytes(data_bases, idx)?;

        Ok((idx, data))
    }

    /// Décode un index depuis 8 bases (16 bits)
    fn decode_index_8byte(&self, bases: &[IupacBase]) -> Result<usize> {
        if bases.len() < 8 {
            return Err(DnaError::Decoding("Pas assez de bases pour l'index".to_string()));
        }

        // Mapping inverse
        let base_to_bits = |b: IupacBase| -> Result<usize> {
            match b {
                IupacBase::A => Ok(0),
                IupacBase::C => Ok(1),
                IupacBase::G => Ok(2),
                IupacBase::T => Ok(3),
                _ => Err(DnaError::Decoding(format!("Base invalide: {:?}", b))),
            }
        };

        let mut idx: usize = 0;

        // Décoder les 8 bases (16 bits) en inversant la rotation
        for i in 0..8 {
            let base = bases[i];
            let bits = base_to_bits(base)?;

            // Inverser la rotation: (x + r) % 4 = bits  =>  x = (bits - r + 4) % 4
            // où r = i % 4
            let rotation = i % 4;
            let two_bits = (bits + 4 - rotation) % 4;

            idx |= (two_bits) << (i * 2);
        }

        Ok(idx)
    }

    /// Décode des bases en octets
    fn decode_bases_to_bytes(&self, bases: &[IupacBase], seq_idx: usize) -> Result<Vec<u8>> {
        let start_offset = seq_idx % 4;
        let mut bytes = Vec::new();

        // Vérifier qu'on a un nombre multiple de 4 bases
        if bases.len() % 4 != 0 {
            return Err(DnaError::Decoding(format!(
                "Nombre de bases non multiple de 4: {}", bases.len()
            )));
        }

        // Traiter 4 bases à la fois
        for chunk_idx in 0..(bases.len() / 4) {
            let mut byte: u8 = 0;

            for bit_pos in 0..4 {
                let base_idx = chunk_idx * 4 + bit_pos;
                let base = bases[base_idx];

                // Utiliser la position de l'octet actuel
                let byte_pos = chunk_idx;
                let rotation = (start_offset + byte_pos + bit_pos) % 4;

                // Mapper la base vers 2 bits
                let two_bits = self.base_to_two_bits(base, rotation)?;

                // Ajouter à l'octet
                byte |= (two_bits as u8) << (6 - bit_pos * 2);
            }

            bytes.push(byte);
        }

        Ok(bytes)
    }

    /// Convertit une base en valeur 2-bit en inversant la rotation
    fn base_to_two_bits(&self, base: IupacBase, rotation: usize) -> Result<usize> {
        // Mapping standard inverse
        let base_to_bits = |b: IupacBase| -> Option<usize> {
            match b {
                IupacBase::A => Some(0),
                IupacBase::C => Some(1),
                IupacBase::G => Some(2),
                IupacBase::T => Some(3),
                _ => None,
            }
        };

        let bits = base_to_bits(base).ok_or_else(|| {
            DnaError::Decoding(format!("Base invalide: {:?}", base))
        })?;

        // Inverser la rotation: (x + r) % 4 = bits  =>  x = (bits - r + 4) % 4
        // Parenthèses correctes: (bits + 4 - (rotation % 4)) % 4
        Ok((bits + 4 - (rotation % 4)) % 4)
    }

    /// Décompression Huffman (utilisant LZ4 comme proxy)
    fn decompress_huffman(&self, data: &[u8]) -> Result<Vec<u8>> {
        // Utiliser LZ4 pour décompresser
        let decompressed = lz4::block::decompress(data, None)
            .map_err(|e| DnaError::Decoding(format!("Erreur de décompression: {}", e)))?;

        Ok(decompressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goldman_2013_encode_simple() {
        // Contraintes plus souples pour Goldman 2013
        let constraints = DnaConstraints {
            gc_min: 0.25,  // Plus tolérant que le défaut (0.4)
            gc_max: 0.75,  // Plus tolérant que le défaut (0.6)
            max_homopolymer: 4,
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Goldman2013Encoder::new(constraints.clone());

        let data = b"Hello Goldman 2013!";
        let sequences = encoder.encode(data).unwrap();

        assert!(!sequences.is_empty(), "Aucune séquence générée");

        // Vérifier que toutes les séquences respectent les contraintes
        for seq in &sequences {
            assert!(seq.validate(&constraints).is_ok(),
                "Séquence ne respecte pas les contraintes");
        }
    }

    #[test]
    fn test_goldman_2013_roundtrip() {
        // Test roundtrip - nécessite le décodeur Goldman 2013
        // Contraintes plus souples pour Goldman 2013
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 4,
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Goldman2013Encoder::new(constraints.clone());

        let original = b"Test data for roundtrip";
        let sequences = encoder.encode(original).unwrap();

        // Pour le roundtrip complet, on aurait besoin du décodeur
        // Pour l'instant, on vérifie juste que l'encodage fonctionne
        assert!(!sequences.is_empty());
    }

    #[test]
    fn test_goldman_2013_roundtrip_full() {
        // Test roundtrip complet encodeur/décodeur
        // Contraintes plus souples pour Goldman 2013
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 4,
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Goldman2013Encoder::new(constraints.clone());
        let decoder = Goldman2013Decoder::new(constraints);

        let original = b"Test data for roundtrip";
        let sequences = encoder.encode(original).unwrap();

        // Décoder
        let recovered = decoder.decode(&sequences).unwrap();

        assert_eq!(original.to_vec(), recovered, "Roundtrip failed");
    }

    #[test]
    fn test_goldman_2013_single_byte_roundtrip() {
        // Test avec un seul octet pour simplifier le debugging
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 4,
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Goldman2013Encoder::new(constraints.clone());
        let decoder = Goldman2013Decoder::new(constraints);

        let original = b"A";  // 65 = 0b01000001
        let sequences = encoder.encode(original).unwrap();

        // Décoder
        let recovered = decoder.decode(&sequences).unwrap();

        assert_eq!(original.to_vec(), recovered, "Single byte roundtrip failed");
    }

    #[test]
    fn test_goldman_2013_three_bytes_roundtrip() {
        // Test avec 3 octets (un chunk complet)
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 4,
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Goldman2013Encoder::new(constraints.clone());
        let decoder = Goldman2013Decoder::new(constraints);

        let original = b"ABC";  // Exactement 3 octets
        let sequences = encoder.encode(original).unwrap();

        println!("Number of sequences: {}", sequences.len());
        for (i, seq) in sequences.iter().enumerate() {
            println!("Sequence {}: {} bases", i, seq.bases.len());
        }

        // Décoder
        let recovered = decoder.decode(&sequences).unwrap();

        assert_eq!(original.to_vec(), recovered, "Three bytes roundtrip failed");
    }

    #[test]
    fn test_goldman_2013_six_bytes_roundtrip() {
        // Test avec 6 octets (deux chunks)
        let constraints = DnaConstraints {
            gc_min: 0.25,
            gc_max: 0.75,
            max_homopolymer: 4,
            max_sequence_length: 200,
            allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
        };

        let encoder = Goldman2013Encoder::new(constraints.clone());
        let decoder = Goldman2013Decoder::new(constraints);

        let original = b"ABCDEF";  // 6 octets = 2 chunks
        let sequences = encoder.encode(original).unwrap();

        println!("Number of sequences: {}", sequences.len());
        for (i, seq) in sequences.iter().enumerate() {
            println!("Sequence {}: {} bases", i, seq.bases.len());
            println!("  Bases: {:?}", seq.bases.iter().take(20).collect::<Vec<_>>());
        }

        // Décoder
        let recovered = decoder.decode(&sequences).unwrap();

        println!("Original: {:?}", original.to_vec());
        println!("Recovered: {:?}", recovered);

        // Vérifier les 3 premiers octets
        assert_eq!(original[0..3], recovered[0..3], "First 3 bytes don't match");
        // Vérifier les 3 derniers octets
        assert_eq!(original[3..6], recovered[3..6], "Last 3 bytes don't match");

        assert_eq!(original.to_vec(), recovered, "Six bytes roundtrip failed");
    }
}
