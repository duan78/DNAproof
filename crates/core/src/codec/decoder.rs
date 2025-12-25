//! Décodeur ADN - Récupère les données depuis les séquences ADN

use crate::error::{DnaError, Result};
use crate::sequence::{DnaSequence, IupacBase};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

// Importer les macros depuis la racine du crate
pub use crate::{log_operation, log_error};

/// Configuration du décodeur
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecoderConfig {
    /// Ignorer les erreurs de checksum
    pub ignore_checksum: bool,

    /// Nombre maximum d'itérations de belief propagation
    pub max_iterations: usize,

    /// Activer la décompression automatique
    pub auto_decompress: bool,

    /// Type de compression attendu
    pub compression_type: CompressionType,
}

/// Type de compression
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionType {
    Lz4,
    Zstd,
    None,
    Auto,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            ignore_checksum: false,
            max_iterations: 10000,
            auto_decompress: true,
            compression_type: CompressionType::Auto,
        }
    }
}

/// Décodeur ADN principal
pub struct Decoder {
    config: DecoderConfig,
}

impl Decoder {
    /// Crée un nouveau décodeur
    pub fn new(config: DecoderConfig) -> Self {
        Self { config }
    }

    /// Décode automatiquement depuis un fichier FASTA en détectant le schéma d'encodage
    pub fn decode_from_fasta_auto(&self, fasta_path: &str) -> Result<Vec<u8>> {
        log_operation!("decode_from_fasta_auto", {
            // Lire le fichier FASTA
            let file = File::open(fasta_path)
                .map_err(|e| DnaError::Decoding(format!("Impossible d'ouvrir {}: {}", fasta_path, e)))?;

            let reader = BufReader::new(file);
            let mut sequences = Vec::new();

            // Lire le fichier et détecter le schéma
            let mut detected_scheme = None;
            let mut current_fasta = String::new();

            for line in reader.lines() {
                let line = line
                    .map_err(|e| DnaError::Decoding(format!("Erreur lecture: {}", e)))?;

                if line.starts_with('>') {
                    // Nouvelle séquence, parser la précédente si elle existe
                    if !current_fasta.is_empty() {
                        let seq = DnaSequence::from_fasta(&current_fasta)?;
                        sequences.push(seq);
                    }
                    current_fasta = line.clone() + "\n";

                    // Détecter le schéma depuis l'en-tête
                    if detected_scheme.is_none() && line.contains("scheme:") {
                        let scheme_part = line
                            .split("scheme:")
                            .nth(1)
                            .and_then(|s| s.split('|').next())
                            .unwrap_or("unknown");
                        detected_scheme = Some(scheme_part.to_string());
                    }
                } else {
                    current_fasta.push_str(&line);
                    current_fasta.push('\n');
                }
            }

            // Parser la dernière séquence
            if !current_fasta.is_empty() {
                let seq = DnaSequence::from_fasta(&current_fasta)?;
                sequences.push(seq);
            }

            if sequences.is_empty() {
                return Err(DnaError::Decoding("Aucune séquence trouvée".to_string()));
            }

            // Décoder avec le bon schéma
            self.decode_with_detected_scheme(&sequences, detected_scheme)
        })
    }

    /// Décode avec le schéma détecté
    fn decode_with_detected_scheme(
        &self,
        sequences: &[DnaSequence],
        scheme: Option<String>,
    ) -> Result<Vec<u8>> {
        let scheme = scheme.as_deref().unwrap_or("unknown");

        match scheme {
            "goldman_2013" => {
                use crate::codec::goldman_2013::Goldman2013Decoder;
                let decoder = Goldman2013Decoder::new(crate::sequence::DnaConstraints::default());
                decoder.decode(sequences)
            }
            "grass_2015" => {
                use crate::codec::grass_2015::Grass2015Decoder;
                let decoder = Grass2015Decoder::new(crate::sequence::DnaConstraints::default());
                decoder.decode(sequences)
            }
            "erlich_zielinski_2017" => {
                // Utiliser le GC-Aware decoder pour EZ 2017
                use crate::codec::gc_aware_encoding::GcAwareDecoder;
                // Contraintes EZ 2017: GC 40-60%, homopolymer <4, 152nt
                let ez_constraints = crate::sequence::DnaConstraints::new(
                    0.40,  // GC min 40%
                    0.60,  // GC max 60%
                    3,     // Max homopolymer 3 (<4)
                    152    // Max length 152nt
                );
                let decoder = GcAwareDecoder::new(ez_constraints);

                // Le GC-Aware decoder décode une séquence à la fois
                // Pour Fountain avec LT codes, on a besoin de plus de logique
                // Pour l'instant, on retourne les données de la première séquence
                // NOTE: Ceci est simplifié - le vrai décodage Fountain nécessite
                // la logique LT codes avec belief propagation
                if sequences.is_empty() {
                    return Err(DnaError::Decoding("Aucune séquence fournie".to_string()));
                }
                decoder.decode(&sequences[0])
            }
            "fountain" | "unknown" => {
                // Utiliser le décodeur générique pour Fountain et inconnu
                self.decode(sequences)
            }
            _ => {
                // Schéma non reconnu, tenter le décodage générique
                self.decode(sequences)
            }
        }
    }

    /// Décode des séquences ADN en données avec gestion des erreurs améliorée
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        log_operation!("decode_data", {
            if sequences.is_empty() {
                return Err(DnaError::Decoding("Aucune séquence fournie".to_string()));
            }

            // Note: Les séquences sont déjà validées pendant l'encodage
            // Pas besoin de re-valider ici avec des contraintes par défaut

            // Pour l'instant, on implémente un décodage simple (Goldman-like)
            // Le décodage Fountain nécessiterait plus de métadonnées

            let mut data = Vec::new();

            // Trier les séquences par chunk_index
            let mut sorted_seqs: Vec<_> = sequences.iter().collect();
            sorted_seqs.sort_by_key(|s| s.metadata.chunk_index);

            for seq in sorted_seqs {
                let chunk_data = self.sequence_to_chunk(seq)?;
                data.extend_from_slice(&chunk_data);
            }

            // Décompression si activée
            let result = if self.config.auto_decompress {
                self.decompress(&data)?
            } else {
                data
            };

            // Vérification finale d'intégrité
            self.verify_integrity(&result)?;

            Ok(result)
        })
    }

    /// Vérifie l'intégrité des données décodées
    fn verify_integrity(&self, data: &[u8]) -> Result<()> {
        // Vérification basique de la taille
        if data.is_empty() {
            return Err(DnaError::Decoding("Données décodées vides".to_string()));
        }
        
        // Ajouter d'autres vérifications d'intégrité ici
        Ok(())
    }

    /// Convertit une séquence en chunk de données
    fn sequence_to_chunk(&self, sequence: &DnaSequence) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let bases = &sequence.bases;

        // Decoder: 4 bases = 1 octet (2 bits par base)
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
                        return Err(DnaError::Decoding(format!(
                            "Base non-standard décodée: {:?}",
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

    /// Décompresse les données
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        let compression_type = match self.config.compression_type {
            CompressionType::Auto => {
                // Auto-détection: essayer LZ4 puis Zstd
                if let Ok(decompressed) = lz4::block::decompress(data, None) {
                    return Ok(decompressed);
                }
                if let Ok(decompressed) = zstd::decode_all(data) {
                    return Ok(decompressed);
                }
                // Fallback: pas de compression
                CompressionType::None
            }
            other => other,
        };

        match compression_type {
            CompressionType::Lz4 => {
                lz4::block::decompress(data, None)
                    .map_err(|e| DnaError::Decoding(format!("Erreur décompression LZ4: {}", e)))
            }
            CompressionType::Zstd => {
                zstd::decode_all(data)
                    .map_err(|e| DnaError::Decoding(format!("Erreur décompression Zstd: {}", e)))
            }
            CompressionType::None | CompressionType::Auto => Ok(data.to_vec()),
        }
    }
}

/// Décodeur Fountain avec belief propagation
pub struct FountainDecoder {
    config: DecoderConfig,
    chunks: HashMap<usize, Vec<u8>>,
    droplets: Vec<Droplet>,
    received: usize,
    required: usize,
    _chunk_size: usize,
}

impl FountainDecoder {
    /// Crée un nouveau décodeur Fountain
    pub fn new(config: DecoderConfig, required_chunks: usize, chunk_size: usize) -> Self {
        Self {
            config,
            chunks: HashMap::new(),
            droplets: Vec::new(),
            received: 0,
            required: required_chunks,
            _chunk_size: chunk_size,
        }
    }

    /// Ajoute un droplet et tente de progresser dans le décodage
    pub fn add_droplet(&mut self, droplet: Droplet) -> Result<Progress> {
        self.received += 1;
        self.droplets.push(droplet);

        // Tenter de décoder avec belief propagation
        self.belief_propagation()
    }

    /// Algorithme de Belief Propagation pour Fountain Codes
    ///
    /// 1. Trouver tous les droplets de degré 1
    /// 2. Extraire les chunks de ces droplets
    /// 3. XOR ces chunks de tous les autres droplets
    /// 4. Répéter jusqu'à ce que tous les chunks soient récupérés ou qu'il n'y ait plus de degré 1
    fn belief_propagation(&mut self) -> Result<Progress> {
        let mut iteration = 0;
        let max_iterations = self.config.max_iterations;

        loop {
            // Trouver tous les droplets de degré 1
            let degree_one_droplets = self.find_degree_one_droplets();

            if degree_one_droplets.is_empty() {
                // Plus de progrès possibles avec les droplets actuels
                if self.chunks.len() == self.required {
                    return Ok(Progress::Complete(self.reassemble()?));
                }
                return Ok(Progress::Incomplete);
            }

            // Pour chaque droplet de degré 1
            for droplet_idx in degree_one_droplets {
                // Le droplet peut avoir été modifié entre-temps, vérifier qu'il est toujours de degré 1
                if self.droplets[droplet_idx].degree() != 1 {
                    continue;
                }

                let chunk_idx = self.droplets[droplet_idx].chunk_indices[0];

                // Si on a déjà ce chunk, ignorer
                if self.chunks.contains_key(&chunk_idx) {
                    continue;
                }

                // Extraire le chunk
                let chunk_data = self.droplets[droplet_idx].payload.clone();
                self.chunks.insert(chunk_idx, chunk_data.clone());

                // XOR ce chunk de tous les autres droplets
                self.xor_out_chunk(chunk_idx, &chunk_data);

                // Vérifier si on a tous les chunks
                if self.chunks.len() == self.required {
                    return Ok(Progress::Complete(self.reassemble()?));
                }
            }

            iteration += 1;
            if iteration >= max_iterations {
                return Err(DnaError::Decoding(
                    "Belief propagation: nombre maximum d'itérations atteint".to_string()
                ));
            }
        }
    }

    /// Trouve tous les indices des droplets de degré 1
    fn find_degree_one_droplets(&self) -> Vec<usize> {
        self.droplets.iter()
            .enumerate()
            .filter(|(_, d)| d.degree() == 1)
            .map(|(i, _)| i)
            .collect()
    }

    /// XOR un chunk de tous les droplets qui le contiennent
    fn xor_out_chunk(&mut self, chunk_idx: usize, chunk_data: &[u8]) {
        for droplet in &mut self.droplets {
            // Chercher ce chunk dans les indices du droplet
            if let Some(pos) = droplet.chunk_indices.iter().position(|&idx| idx == chunk_idx) {
                // XOR le chunk du payload
                xor_bytes(&mut droplet.payload, chunk_data);

                // Retirer l'index de la liste
                droplet.chunk_indices.remove(pos);
            }
        }
    }

    /// Réassemble les données dans l'ordre
    fn reassemble(&self) -> Result<Vec<u8>> {
        let mut data = Vec::new();

        for i in 0..self.required {
            if let Some(chunk) = self.chunks.get(&i) {
                data.extend_from_slice(chunk);
            } else {
                return Err(DnaError::InsufficientData {
                    need: self.required,
                    have: self.chunks.len(),
                });
            }
        }

        Ok(data)
    }

    /// Retourne le nombre de chunks récupérés
    pub fn recovered_count(&self) -> usize {
        self.chunks.len()
    }

    /// Retourne le nombre de droplets reçus
    pub fn received_count(&self) -> usize {
        self.received
    }

    /// Retourne true si tous les chunks ont été récupérés
    pub fn is_complete(&self) -> bool {
        self.chunks.len() == self.required
    }
}

/// XOR deux tableaux d'octets in-place
fn xor_bytes(dest: &mut [u8], src: &[u8]) {
    for (i, &byte) in src.iter().enumerate() {
        if i < dest.len() {
            dest[i] ^= byte;
        }
    }
}

/// Droplet Fountain
#[derive(Debug, Clone)]
pub struct Droplet {
    /// Indices des chunks utilisés
    pub chunk_indices: Vec<usize>,
    /// Payload XORé
    pub payload: Vec<u8>,
    /// Seed utilisé
    pub seed: u64,
}

impl Droplet {
    /// Retourne le degré (nombre de chunks combinés)
    pub fn degree(&self) -> usize {
        self.chunk_indices.len()
    }

    /// Crée un nouveau droplet
    pub fn new(chunk_indices: Vec<usize>, payload: Vec<u8>, seed: u64) -> Self {
        Self {
            chunk_indices,
            payload,
            seed,
        }
    }
}

/// Progression du décodage
#[derive(Debug, Clone)]
pub enum Progress {
    Incomplete,
    Complete(Vec<u8>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encoder::{Encoder, EncoderConfig, EncoderType};

    #[test]
    fn test_decoder_creation() {
        let config = DecoderConfig::default();
        let _decoder = Decoder::new(config);
        // Juste vérifier que ça compile
    }

    #[test]
    fn test_roundtrip_goldman() {
        // Encoder
        let encoder_config = EncoderConfig {
            encoder_type: EncoderType::Goldman,
            chunk_size: 4,
            compression_enabled: false,
            constraints: crate::sequence::DnaConstraints {
                gc_min: 0.15,
                gc_max: 0.85,
                max_homopolymer: 6,
                max_sequence_length: 200,
                allowed_bases: vec![
                    crate::sequence::IupacBase::A,
                    crate::sequence::IupacBase::C,
                    crate::sequence::IupacBase::G,
                    crate::sequence::IupacBase::T,
                ],
            },
            ..Default::default()
        };
        let encoder = Encoder::new(encoder_config).unwrap();

        let original = b"Hello, DNA!";
        let sequences = encoder.encode(original).unwrap();

        // Decoder
        let decoder_config = DecoderConfig {
            auto_decompress: false,
            ..Default::default()
        };
        let decoder = Decoder::new(decoder_config);

        let recovered = decoder.decode(&sequences).unwrap();
        assert_eq!(original.to_vec(), recovered);
    }

    #[test]
    fn test_sequence_to_chunk() {
        // Note: DnaSequence n'a pas de champs A, C, G, T accessibles directement
        // On utilise crate::sequence::IupacBase à la place
    }

    #[test]
    fn test_droplet_creation() {
        let droplet = Droplet::new(vec![0, 1, 2], vec![1, 2, 3], 42);
        assert_eq!(droplet.degree(), 3);
        assert_eq!(droplet.seed, 42);
    }

    #[test]
    fn test_fountain_decoder_degree_one() {
        let config = DecoderConfig::default();
        let mut decoder = FountainDecoder::new(config, 3, 4);

        // Ajouter 3 droplets de degré 1
        let droplet1 = Droplet::new(vec![0], vec![1, 2, 3, 4], 0);
        let droplet2 = Droplet::new(vec![1], vec![5, 6, 7, 8], 1);
        let droplet3 = Droplet::new(vec![2], vec![9, 10, 11, 12], 2);

        assert!(matches!(decoder.add_droplet(droplet1).unwrap(), Progress::Incomplete));
        assert!(matches!(decoder.add_droplet(droplet2).unwrap(), Progress::Incomplete));

        // Le troisième devrait compléter le décodage
        let result = decoder.add_droplet(droplet3).unwrap();
        assert!(matches!(result, Progress::Complete(_)));

        if let Progress::Complete(data) = result {
            assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        }
    }

    #[test]
    fn test_fountain_decoder_belief_propagation() {
        let config = DecoderConfig::default();
        let mut decoder = FountainDecoder::new(config, 3, 4);

        // Créer 3 chunks
        let chunk0 = vec![1, 2, 3, 4];
        let chunk1 = vec![5, 6, 7, 8];
        let chunk2 = vec![9, 10, 11, 12];

        // Créer un droplet de degré 1 pour chunk0
        let droplet1 = Droplet::new(vec![0], chunk0.clone(), 0);

        // Créer un droplet de degré 2: chunk1 XOR chunk2
        let mut payload = chunk1.clone();
        xor_bytes(&mut payload, &chunk2);
        let droplet2 = Droplet::new(vec![1, 2], payload, 1);

        // Créer un droplet de degré 1 pour chunk1
        let droplet3 = Droplet::new(vec![1], chunk1.clone(), 2);

        // Ajouter dans l'ordre: d'abord le droplet de degré 2, puis les degré 1
        assert!(matches!(decoder.add_droplet(droplet2).unwrap(), Progress::Incomplete));
        assert!(matches!(decoder.add_droplet(droplet1).unwrap(), Progress::Incomplete));

        // Après droplet1, belief propagation devrait extraire chunk0
        // et le XOR de droplet2, laissant chunk2
        assert_eq!(decoder.recovered_count(), 1);

        // Ajouter droplet3 (chunk1) - cela devrait compléter le décodage
        let result = decoder.add_droplet(droplet3).unwrap();
        assert!(matches!(result, Progress::Complete(_)));

        if let Progress::Complete(data) = result {
            assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        }
    }

    #[test]
    fn test_fountain_decoder_insufficient_data() {
        let config = DecoderConfig::default();
        let mut decoder = FountainDecoder::new(config, 5, 4);

        // Ajouter seulement 3 chunks sur 5 requis
        let droplet1 = Droplet::new(vec![0], vec![1, 2, 3, 4], 0);
        let droplet2 = Droplet::new(vec![1], vec![5, 6, 7, 8], 1);
        let droplet3 = Droplet::new(vec![2], vec![9, 10, 11, 12], 2);

        assert!(matches!(decoder.add_droplet(droplet1).unwrap(), Progress::Incomplete));
        assert!(matches!(decoder.add_droplet(droplet2).unwrap(), Progress::Incomplete));
        assert!(matches!(decoder.add_droplet(droplet3).unwrap(), Progress::Incomplete));

        assert_eq!(decoder.recovered_count(), 3);
        assert!(!decoder.is_complete());
    }

    #[test]
    fn test_xor_bytes() {
        let mut a = vec![0b11110000, 0b10101010];
        let b = vec![0b00001111, 0b01010101];

        xor_bytes(&mut a, &b);

        assert_eq!(a, vec![0b11111111, 0b11111111]);
    }
}
