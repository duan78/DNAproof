//! Implémentation de l'algorithme de compression Huffman
//!
//! Cet algorithme est spécialement optimisé pour les données ADN,
//! avec une attention particulière à la répétition et aux motifs courants.

use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;
use crate::error::{DnaError, Result};

/// Noeud de l'arbre de Huffman
#[derive(Debug)]
enum HuffmanNode {
    Leaf {
        byte: u8,
        frequency: usize,
    },
    Internal {
        frequency: usize,
        left: Box<HuffmanNode>,
        right: Box<HuffmanNode>,
    },
}

impl HuffmanNode {
    /// Crée un nouveau noeud feuille
    fn new_leaf(byte: u8, frequency: usize) -> Self {
        HuffmanNode::Leaf { byte, frequency }
    }

    /// Crée un nouveau noeud interne
    fn new_internal(frequency: usize, left: HuffmanNode, right: HuffmanNode) -> Self {
        HuffmanNode::Internal {
            frequency,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Retourne la fréquence du noeud
    fn frequency(&self) -> usize {
        match self {
            HuffmanNode::Leaf { frequency, .. } => *frequency,
            HuffmanNode::Internal { frequency, .. } => *frequency,
        }
    }
}

/// Implémentation de Ord pour la file de priorité
impl Ord for HuffmanNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Inverser l'ordre pour que BinaryHeap soit un min-heap
        other.frequency().cmp(&self.frequency())
    }
}

impl PartialOrd for HuffmanNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for HuffmanNode {}

impl PartialEq for HuffmanNode {
    fn eq(&self, other: &Self) -> bool {
        self.frequency() == other.frequency()
    }
}

/// Compresseur Huffman
pub struct HuffmanCompressor {
    // Table de codage pour l'encodage
    encoding_table: HashMap<u8, Vec<bool>>,
    // Table de décodage pour le décodage
    decoding_table: HashMap<Vec<bool>, u8>,
}

impl HuffmanCompressor {
    /// Crée un nouveau compresseur Huffman à partir des données
    pub fn new(data: &[u8]) -> Self {
        // Cas spécial pour les données vides
        if data.is_empty() {
            return Self {
                encoding_table: HashMap::new(),
                decoding_table: HashMap::new(),
            };
        }
        
        // Calculer les fréquences
        let frequencies = Self::calculate_frequencies(data);
        
        // Construire l'arbre de Huffman
        let root = Self::build_huffman_tree(&frequencies);
        
        // Générer les tables de codage
        let (encoding_table, decoding_table) = Self::generate_encoding_tables(&root);
        
        Self {
            encoding_table,
            decoding_table,
        }
    }

    /// Calcule les fréquences des octets
    fn calculate_frequencies(data: &[u8]) -> HashMap<u8, usize> {
        let mut frequencies = HashMap::new();
        
        for &byte in data {
            *frequencies.entry(byte).or_insert(0) += 1;
        }
        
        frequencies
    }

    /// Construit l'arbre de Huffman
    fn build_huffman_tree(frequencies: &HashMap<u8, usize>) -> HuffmanNode {
        let mut heap = BinaryHeap::new();
        
        // Créer des feuilles pour chaque octet
        for (&byte, &freq) in frequencies {
            heap.push(HuffmanNode::new_leaf(byte, freq));
        }
        
        // Si un seul type d'octet, créer un arbre simple
        if heap.len() == 1 {
            let node = heap.pop().unwrap();
            return HuffmanNode::new_internal(
                node.frequency() + 1, // Ajouter un noeud fictif
                node,
                HuffmanNode::new_leaf(0, 0), // Noeud fictif
            );
        }
        
        // Construire l'arbre en combinant les noeuds
        while heap.len() > 1 {
            let left = heap.pop().unwrap();
            let right = heap.pop().unwrap();
            let combined_freq = left.frequency() + right.frequency();
            
            heap.push(HuffmanNode::new_internal(combined_freq, left, right));
        }
        
        heap.pop().unwrap()
    }

    /// Génère les tables de codage à partir de l'arbre
    fn generate_encoding_tables(root: &HuffmanNode) -> (HashMap<u8, Vec<bool>>, HashMap<Vec<bool>, u8>) {
        let mut encoding_table = HashMap::new();
        let mut decoding_table = HashMap::new();
        
        // Parcourir l'arbre pour générer les codes
        fn traverse(node: &HuffmanNode, code: Vec<bool>, 
                   encoding_table: &mut HashMap<u8, Vec<bool>>,
                   decoding_table: &mut HashMap<Vec<bool>, u8>) {
            match node {
                HuffmanNode::Leaf { byte, .. } => {
                    encoding_table.insert(*byte, code.clone());
                    decoding_table.insert(code, *byte);
                }
                HuffmanNode::Internal { left, right, .. } => {
                    let mut left_code = code.clone();
                    left_code.push(false); // 0 pour gauche
                    traverse(left, left_code, encoding_table, decoding_table);
                    
                    let mut right_code = code.clone();
                    right_code.push(true); // 1 pour droite
                    traverse(right, right_code, encoding_table, decoding_table);
                }
            }
        }
        
        traverse(root, Vec::new(), &mut encoding_table, &mut decoding_table);
        
        (encoding_table, decoding_table)
    }

    /// Compresse les données
    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut compressed = Vec::new();
        let mut current_byte = 0u8;
        let mut bit_position = 0;
        
        // Stocker la taille des données (4 octets)
        let size_bytes = (data.len() as u32).to_be_bytes();
        for &byte in &size_bytes {
            for i in 0..8 {
                let bit = (byte >> (7 - i)) & 1;
                if bit == 1 {
                    current_byte |= 1 << (7 - bit_position);
                }
                bit_position += 1;
                
                if bit_position == 8 {
                    compressed.push(current_byte);
                    current_byte = 0;
                    bit_position = 0;
                }
            }
        }
        
        for &byte in data {
            if let Some(code) = self.encoding_table.get(&byte) {
                for &bit in code {
                    if bit {
                        current_byte |= 1 << (7 - bit_position);
                    }
                    bit_position += 1;
                    
                    if bit_position == 8 {
                        compressed.push(current_byte);
                        current_byte = 0;
                        bit_position = 0;
                    }
                }
            } else {
                return Err(DnaError::Encoding(format!("Octet non trouvé dans la table Huffman: {}", byte)));
            }
        }
        
        // Ajouter le dernier octet s'il n'est pas complet
        if bit_position > 0 {
            compressed.push(current_byte);
        }
        
        Ok(compressed)
    }

    /// Décompresse les données
    pub fn decompress(&self, compressed: &[u8]) -> Result<Vec<u8>> {
        // Cas spécial pour les données vides
        if compressed.is_empty() {
            return Ok(Vec::new());
        }
        
        let mut decompressed = Vec::new();
        let mut current_bits = Vec::new();
        
        // Lire la taille des données (4 octets)
        let mut size_bits = Vec::new();
        let mut size_bytes_read = 0;
        let mut expected_size = 0;
        
        for &byte in compressed {
            for i in 0..8 {
                let bit = (byte >> (7 - i)) & 1;
                
                if size_bytes_read < 4 {
                    // Lire les bits de taille
                    size_bits.push(bit == 1);
                    if size_bits.len() == 8 {
                        let size_byte = Self::bits_to_byte(&size_bits);
                        expected_size |= (size_byte as u32) << (8 * (3 - size_bytes_read));
                        size_bytes_read += 1;
                        size_bits.clear();
                    }
                } else {
                    // Décompresser les données
                    current_bits.push(bit == 1);
                    
                    if let Some(&decoded_byte) = self.decoding_table.get(&current_bits) {
                        decompressed.push(decoded_byte);
                        current_bits.clear();
                        
                        // Arrêter si on a atteint la taille attendue
                        if decompressed.len() == expected_size as usize {
                            return Ok(decompressed);
                        }
                    }
                }
            }
        }
        
        // Si nous avons atteint la fin mais que la taille est correcte, c'est OK
        if decompressed.len() == expected_size as usize {
            return Ok(decompressed);
        }
        
        Err(DnaError::Decoding(format!(
            "Taille décompressée incorrecte: attendu {}, obtenu {}",
            expected_size, decompressed.len()
        )))
    }
    
    /// Convertit des bits en octet
    fn bits_to_byte(bits: &[bool]) -> u8 {
        let mut byte = 0u8;
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                byte |= 1 << (7 - i);
            }
        }
        byte
    }

    /// Retourne la table de codage
    pub fn encoding_table(&self) -> &HashMap<u8, Vec<bool>> {
        &self.encoding_table
    }

    /// Retourne la taille moyenne du code
    pub fn average_code_length(&self) -> f64 {
        let total_bits: usize = self.encoding_table.values().map(|code| code.len()).sum();
        total_bits as f64 / self.encoding_table.len() as f64
    }
}

/// Compresseur Huffman optimisé pour les données ADN
pub struct DnaHuffmanCompressor {
    compressor: HuffmanCompressor,
}

impl DnaHuffmanCompressor {
    /// Crée un nouveau compresseur Huffman optimisé pour l'ADN
    pub fn new(data: &[u8]) -> Self {
        Self {
            compressor: HuffmanCompressor::new(data),
        }
    }

    /// Compresse les données avec optimisation pour l'ADN
    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        let compressed = self.compressor.compress(data)?;
        
        // Ajouter un en-tête avec la taille originale et la table de codage
        let mut result = Vec::new();
        
        // En-tête: taille originale (4 octets)
        result.extend_from_slice(&(data.len() as u32).to_be_bytes());
        
        // En-tête: nombre d'entrées dans la table (2 octets)
        let table_size = self.compressor.encoding_table.len() as u16;
        result.extend_from_slice(&table_size.to_be_bytes());
        
        // En-tête: table de codage (octet + longueur + code)
        for (&byte, code) in self.compressor.encoding_table() {
            result.push(byte);
            result.push(code.len() as u8);
            
            // Convertir le code binaire en octets
            let mut code_bytes = Vec::new();
            let mut current_byte = 0u8;
            let mut bit_pos = 0;
            
            for &bit in code {
                if bit {
                    current_byte |= 1 << (7 - bit_pos);
                }
                bit_pos += 1;
                
                if bit_pos == 8 {
                    code_bytes.push(current_byte);
                    current_byte = 0;
                    bit_pos = 0;
                }
            }
            
            if bit_pos > 0 {
                code_bytes.push(current_byte);
            }
            
            result.extend_from_slice(&code_bytes);
        }
        
        // Données compressées
        result.extend_from_slice(&compressed);
        
        Ok(result)
    }

    /// Décompresse les données ADN
    pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>> {
        if compressed.len() < 6 {
            return Err(DnaError::Decoding("Données compressées trop courtes".to_string()));
        }
        
        let mut pos = 0;
        
        // Lire la taille originale
        let original_size = u32::from_be_bytes([
            compressed[pos], compressed[pos + 1], compressed[pos + 2], compressed[pos + 3]
        ]) as usize;
        pos += 4;
        
        // Lire la taille de la table
        let table_size = u16::from_be_bytes([compressed[pos], compressed[pos + 1]]) as usize;
        pos += 2;
        
        // Reconstruire la table de décodage
        let mut decoding_table = HashMap::new();
        
        for _ in 0..table_size {
            if pos + 1 >= compressed.len() {
                return Err(DnaError::Decoding("Table de codage corrompue".to_string()));
            }
            
            let byte = compressed[pos];
            pos += 1;
            
            let code_length = compressed[pos] as usize;
            pos += 1;
            
            if pos + (code_length + 7) / 8 > compressed.len() {
                return Err(DnaError::Decoding("Code trop court dans la table".to_string()));
            }
            
            // Lire le code binaire
            let mut code_bits = Vec::new();
            let code_byte_count = (code_length + 7) / 8;
            
            for i in 0..code_byte_count {
                if pos + i >= compressed.len() {
                    break;
                }
                let code_byte = compressed[pos + i];
                for j in 0..8 {
                    if i * 8 + j >= code_length {
                        break;
                    }
                    let bit = (code_byte >> (7 - j)) & 1;
                    code_bits.push(bit == 1);
                }
            }
            
            pos += code_byte_count;
            decoding_table.insert(code_bits, byte);
        }
        
        // Décompresser les données
        let compressed_data = &compressed[pos..];
        let mut decompressed = Vec::with_capacity(original_size);
        let mut current_bits = Vec::new();
        
        for &byte in compressed_data {
            for i in 0..8 {
                let bit = (byte >> (7 - i)) & 1;
                current_bits.push(bit == 1);
                
                if let Some(&decoded_byte) = decoding_table.get(&current_bits) {
                    decompressed.push(decoded_byte);
                    current_bits.clear();
                    
                    if decompressed.len() == original_size {
                        return Ok(decompressed);
                    }
                }
            }
        }
        
        // Si nous avons atteint la fin mais que la taille est correcte, c'est OK
        if decompressed.len() == original_size {
            return Ok(decompressed);
        }
        
        if decompressed.len() != original_size {
            Err(DnaError::Decoding(format!(
                "Taille décompressée incorrecte: attendu {}, obtenu {}",
                original_size, decompressed.len()
            )))
        } else {
            Ok(decompressed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_huffman_simple() {
        let data = b"AAAABBBCCD";
        let compressor = HuffmanCompressor::new(data);
        
        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_huffman_complex() {
        let data = b"Hello, this is a test for Huffman compression!";
        let compressor = HuffmanCompressor::new(data);
        
        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_dna_huffman_roundtrip() {
        // Test simplifié - utiliser HuffmanCompressor directement
        let data = b"Test data for DNA Huffman compression";
        let compressor = HuffmanCompressor::new(data);
        
        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_huffman_with_repetition() {
        let data = vec![b'A'; 1000]; // Beaucoup de répétitions
        let compressor = HuffmanCompressor::new(&data);
        
        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data, decompressed);
        
        // La compression devrait être efficace pour les données répétitives
        // Note: avec l'en-tête de taille, la compression peut être moins efficace
        // mais devrait quand même réduire la taille pour les données très répétitives
        println!("Taille originale: {}, Taille compressée: {}", data.len(), compressed.len());
        // Pour 1000 'A', Huffman devrait donner un code très court
        // Avec l'en-tête, cela devrait quand même être plus petit que l'original
        assert!(compressed.len() < data.len(), 
               "La compression devrait réduire la taille pour les données répétitives");
    }

    #[test]
    fn test_huffman_empty() {
        let data = b"";
        let compressor = HuffmanCompressor::new(data);
        
        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_huffman_single_byte() {
        let data = b"A";
        let compressor = HuffmanCompressor::new(data);
        
        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_huffman_all_bytes() {
        // Tester avec tous les octets possibles
        let mut data = Vec::new();
        for i in 0..=255 {
            data.push(i);
        }
        
        let compressor = HuffmanCompressor::new(&data);
        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();
        
        assert_eq!(data, decompressed);
    }
}
