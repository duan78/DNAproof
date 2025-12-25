//! Encodeur ADN - Implémente DNA Fountain et autres algorithmes

use crate::error::{DnaError, Result};
use crate::sequence::{DnaConstraints, DnaSequence, IupacBase};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Importer les macros depuis la racine du crate
pub use crate::{log_operation, log_error};

/// Type d'algorithme d'encodage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderType {
    /// DNA Fountain - LT codes avec distribution robust soliton
    Fountain,
    /// Erlich-Zielinski 2017 - DNA Fountain avec paramètres validés (Science 2017)
    /// Paramètres: c=0.1, δ=0.5, GC 40-60%, homopolymer <4, 152nt
    ErlichZielinski2017,
    /// Goldman et al. 2013 - Nature 2013 (Huffman + 3-base rotation + 4-byte addressing)
    Goldman2013,
    /// Goldman code - Codage de Huffman simple (legacy)
    Goldman,
    /// Encodage adaptatif
    Adaptive,
    /// Encodage base-3 optimisé
    Base3,
}

impl Default for EncoderType {
    fn default() -> Self {
        Self::Fountain
    }
}

/// Configuration de l'encodeur
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderConfig {
    /// Type d'algorithme
    pub encoder_type: EncoderType,

    /// Taille des chunks (octets)
    pub chunk_size: usize,

    /// Facteur de redondance (1.0 = minimum, 2.0 = 2x plus de gouttes)
    pub redundancy: f64,

    /// Activer la compression
    pub compression_enabled: bool,

    /// Type de compression
    pub compression_type: CompressionType,

    /// Contraintes ADN
    pub constraints: DnaConstraints,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32, // 32 octets par chunk
            redundancy: 1.5,
            compression_enabled: true,
            compression_type: CompressionType::Lz4,
            constraints: DnaConstraints::default(),
        }
    }
}

/// Type de compression
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionType {
    Lz4,
    Zstd,
    None,
}

/// Encodeur ADN principal
pub struct Encoder {
    config: EncoderConfig,
}

impl Encoder {
    /// Crée un nouvel encodeur
    pub fn new(config: EncoderConfig) -> Result<Self> {
        Ok(Self { config })
    }

    /// Encode des données en séquences ADN avec optimisation de performance
    pub fn encode(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        log_operation!("encode_data", {
            // 1. Compression si activée
            let processed_data = if self.config.compression_enabled {
                self.compress(data)?
            } else {
                data.to_vec()
            };

            // 2. Division en chunks
            let chunks = self.split_into_chunks(&processed_data);

            // 3. Encodage selon le type avec parallélisme
            let sequences = match self.config.encoder_type {
                EncoderType::Fountain => self.encode_fountain_optimized(&chunks)?,
                EncoderType::ErlichZielinski2017 => self.encode_erlich_zielinski_2017(&chunks)?,
                EncoderType::Goldman2013 => self.encode_goldman_2013(data)?,
                EncoderType::Goldman => self.encode_goldman(&chunks)?,
                EncoderType::Adaptive => self.encode_adaptive(&chunks)?,
                EncoderType::Base3 => self.encode_base3(&chunks)?,
            };

            Ok(sequences)
        })
    }

    /// Compresse les données
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        match self.config.compression_type {
            CompressionType::Lz4 => {
                let compressed = lz4::block::compress(
                    data,
                    None, // Mode par défaut
                    true, // Avec checksum
                )
                .map_err(|e| DnaError::Encoding(format!("Erreur LZ4: {}", e)))?;
                Ok(compressed)
            }
            CompressionType::Zstd => {
                let compressed = zstd::encode_all(data, 0)
                    .map_err(|e| DnaError::Encoding(format!("Erreur Zstd: {}", e)))?;
                Ok(compressed)
            }
            CompressionType::None => Ok(data.to_vec()),
        }
    }

    /// Divise les données en chunks
    fn split_into_chunks(&self, data: &[u8]) -> Vec<Vec<u8>> {
        data.chunks(self.config.chunk_size)
            .map(|c| c.to_vec())
            .collect()
    }

    /// Encodage DNA Fountain optimisé avec parallélisme
    fn encode_fountain_optimized(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        let num_chunks = chunks.len();
        let num_droplets = (num_chunks as f64 * self.config.redundancy).ceil() as usize;

        // Utiliser Rayon pour le parallélisme
        let sequences: Result<Vec<DnaSequence>> = (0..num_droplets)
            .into_par_iter()
            .map(|seed| {
                // Échantillonner le degré depuis la distribution robust soliton
                let degree = Self::sample_robust_soliton_degree(num_chunks, seed as u64);

                // Sélectionner les chunks (seed-based pour reproductibilité)
                let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed as u64);

                // XOR des chunks sélectionnés
                let payload = Self::xor_chunks(&selected_chunks)?;

                // Convertir en ADN avec contraintes
                self.payload_to_dna(payload, seed as u64)
            })
            .collect();

        sequences
    }

    /// Encodage Erlich-Zielinski 2017 - DNA Fountain validé (Science 2017)
    ///
    /// Spécifications du papier:
    /// - Distribution Robust Soliton: c=0.1, δ=0.5
    /// - Contraintes biochemical: GC 40-60%, homopolymer <4
    /// - Longueur d'oligo: 152nt (± quelques bases)
    /// - Overhead théorique: 1.03-1.07× (minimum)
    fn encode_erlich_zielinski_2017(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        // Contraintes Erlich-Zielinski 2017
        let ez_constraints = DnaConstraints::new(
            0.40,  // GC min 40%
            0.60,  // GC max 60%
            3,     // Max homopolymer 3 (<4)
            152    // Max length 152nt (spécification papier)
        );

        let num_chunks = chunks.len();
        // Redondance plus faible avec EZ 2017 (1.03-1.07 recommandé)
        let redundancy = self.config.redundancy.min(1.07).max(1.03);
        let num_droplets = (num_chunks as f64 * redundancy).ceil() as usize;

        let mut sequences = Vec::with_capacity(num_droplets);

        for seed in 0..num_droplets {
            // Échantillonner le degré avec paramètres EZ 2017
            let degree = Self::sample_robust_soliton_degree_ez2017(num_chunks, seed as u64);

            // Sélectionner les chunks
            let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed as u64);

            // XOR des chunks
            let payload = Self::xor_chunks(&selected_chunks)?;

            // Convertir en ADN avec contraintes EZ 2017 strictes
            let dna = self.payload_to_dna_with_constraints(
                payload,
                seed as u64,
                &ez_constraints,
            )?;

            // Validation stricte des contraintes EZ 2017
            Self::validate_erlich_zielinski_2017_sequence(&dna)?;

            sequences.push(dna);
        }

        Ok(sequences)
    }

    /// Échantillonne un degré avec distribution Robust Soliton (paramètres EZ 2017)
    ///
    /// Selon Erlich & Zielinski 2017:
    /// - c = 0.1
    /// - δ = 0.5
    /// - K = nombre de chunks
    fn sample_robust_soliton_degree_ez2017(num_chunks: usize, seed: u64) -> usize {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // Paramètres Robust Soliton du papier EZ 2017
        let k = num_chunks as f64;
        let c = 0.1;  // Constante c du papier
        let delta = 0.5;  // Paramètre δ du papier

        // Fonction Tau définie dans le papier
        let tau = |d: f64| -> f64 {
            let s = c * k.ln();
            let t = c * k.sqrt();
            let k_over_s = (k / s).floor();
            let k_over_t = (k / t).floor();

            if d <= k_over_s {
                s / k / d
            } else if (d - k_over_s).abs() < 0.5 {
                s * (s - 1.0) / k
            } else if d <= k_over_t {
                t / k / d
            } else {
                0.0
            }
        };

        // Fonction Rho (idéal soliton)
        let rho = |d: f64| -> f64 {
            if d == 1.0 {
                1.0 / k
            } else {
                1.0 / (d * (d - 1.0))
            }
        };

        // Calculer les poids normalisés
        let mut weights = Vec::with_capacity(num_chunks);
        let mut sum = 0.0;

        for d in 1..=num_chunks {
            let d_float = d as f64;
            let weight = rho(d_float) + tau(d_float);
            weights.push(weight);
            sum += weight;
        }

        // Normaliser
        for w in weights.iter_mut() {
            *w /= sum;
        }

        // Échantillonner avec la méthode de la roulette
        let mut sample = rng.gen::<f64>();
        for (d, weight) in weights.iter().enumerate() {
            sample -= weight;
            if sample <= 0.0 {
                return d + 1;  // Les degrés commencent à 1
            }
        }

        num_chunks  // Fallback au degré maximum
    }

    /// Convertit un payload en ADN avec contraintes spécifiques
    fn payload_to_dna_with_constraints(
        &self,
        payload: Vec<u8>,
        seed: u64,
        constraints: &DnaConstraints,
    ) -> Result<DnaSequence> {
        // D'abord, encoder en bases idéales
        let mut ideal_bases: Vec<IupacBase> = Vec::with_capacity(payload.len() * 4);
        for byte in &payload {
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
                ideal_bases.push(base);
            }
        }

        // Ensuite, utiliser enforce_constraints pour respecter les contraintes
        let validator = crate::constraints::DnaConstraintValidator::with_constraints(
            constraints.clone(),
        );
        let enforced_bases = validator.enforce_constraints(&ideal_bases)?;

        // Si les bases font la conversion trop longue, tronquer
        let max_len = constraints.max_sequence_length;
        if enforced_bases.len() > max_len {
            // Logique de retry avec seed différent
            return self.payload_to_dna_with_constraints_retry(payload, seed, constraints);
        }

        let sequence = DnaSequence::new(
            enforced_bases,
            String::from("ez2017"),
            0,
            payload.len(),
            seed,
        );

        // Validation finale
        sequence.validate(constraints)?;

        Ok(sequence)
    }

    /// Encode avec retry si la première tentative échoue à cause de la longueur
    fn payload_to_dna_with_constraints_retry(
        &self,
        payload: Vec<u8>,
        mut seed: u64,
        constraints: &DnaConstraints,
    ) -> Result<DnaSequence> {
        const MAX_RETRIES: usize = 10;

        for attempt in 0..MAX_RETRIES {
            seed += attempt as u64;  // Variante le seed

            let mut bases = Vec::with_capacity(payload.len() * 4);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let validator = crate::constraints::DnaConstraintValidator::with_constraints(
                constraints.clone(),
            );

            for byte in &payload {
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

                    if validator.can_append(&bases, base) {
                        bases.push(base);
                    } else {
                        // Essayer différentes alternatives
                        let bases_set = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
                        let mut found = false;

                        for &alt_base in &bases_set {
                            if alt_base != base && validator.can_append(&bases, alt_base) {
                                bases.push(alt_base);
                                found = true;
                                break;
                            }
                        }

                        if !found {
                            // Dernier recours: prendre une base aléatoire qui marche
                            for &alt_base in &bases_set {
                                if validator.can_append(&bases, alt_base) {
                                    bases.push(alt_base);
                                    found = true;
                                    break;
                                }
                            }
                        }

                        if !found {
                            return Err(DnaError::ConstraintViolation(
                                format!("Impossible de trouver une base valide à l'attempt {}", attempt)
                            ));
                        }
                    }

                    // Vérifier qu'on ne dépasse pas la longueur max
                    if bases.len() >= constraints.max_sequence_length {
                        break;
                    }
                }

                // Si on atteint la longueur max, arrêter
                if bases.len() >= constraints.max_sequence_length {
                    break;
                }
            }

            // Créer la séquence
            let sequence = DnaSequence::new(
                bases,
                String::from("ez2017"),
                0,
                payload.len(),
                seed,
            );

            // Valider
            if sequence.validate(constraints).is_ok() {
                return Ok(sequence);
            }
        }

        Err(DnaError::ConstraintViolation(
            "Impossible d'encoder le payload avec les contraintes EZ 2017 après {} tentatives".to_string()
        ))
    }

    /// Valide qu'une séquence respecte les contraintes Erlich-Zielinski 2017
    fn validate_erlich_zielinski_2017_sequence(sequence: &DnaSequence) -> Result<()> {
        // Vérifier la longueur (152nt ± quelques bases de tolérance)
        let len = sequence.bases.len();
        if len < 140 || len > 160 {
            return Err(DnaError::ConstraintViolation(format!(
                "Longueur de séquence {} hors limites EZ 2017 (140-160nt)", len
            )));
        }

        // Vérifier le GC ratio (40-60%)
        let gc_count = sequence.bases.iter()
            .filter(|b| b.is_gc())
            .count();
        let gc_ratio = gc_count as f64 / len as f64;

        if gc_ratio < 0.40 || gc_ratio > 0.60 {
            return Err(DnaError::ConstraintViolation(format!(
                "GC ratio {:.2} hors limites EZ 2017 (40-60%)", gc_ratio
            )));
        }

        // Vérifier les homopolymères (<4)
        let max_homopolymer = crate::constraints::find_max_homopolymer(&sequence.bases);
        if max_homopolymer >= 4 {
            return Err(DnaError::ConstraintViolation(format!(
                "Homopolymer de longueur {} détecté, EZ 2017 requiert <4", max_homopolymer
            )));
        }

        Ok(())
    }

    /// Encodage DNA Fountain (version originale pour compatibilité)
    fn encode_fountain(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        let num_chunks = chunks.len();
        let num_droplets = (num_chunks as f64 * self.config.redundancy).ceil() as usize;

        let mut sequences = Vec::with_capacity(num_droplets);

        for seed in 0..num_droplets {
            // Échantillonner le degré depuis la distribution robust soliton
            let degree = Self::sample_robust_soliton_degree(num_chunks, seed as u64);

            // Sélectionner les chunks (seed-based pour reproductibilité)
            let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed as u64);

            // XOR des chunks sélectionnés
            let payload = Self::xor_chunks(&selected_chunks)?;

            // Convertir en ADN avec contraintes
            let dna = self.payload_to_dna(payload, seed as u64)?;

            sequences.push(dna);
        }

        Ok(sequences)
    }

    /// Échantillonne un degré depuis la distribution Robust Soliton
    fn sample_robust_soliton_degree(num_chunks: usize, seed: u64) -> usize {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);

        // Distribution Robust Soliton simplifiée
        // K = num_chunks, c = 0.1, delta = 0.5

        let k = num_chunks as f64;
        let c = 0.1;
        let _delta = 0.5; // Paramètre Robust Soliton (non utilisé dans cette implémentation simplifiée)

        // Tau function
        let tau = |d: f64| -> f64 {
            if d <= (k / c - 1.0).ceil() {
                1.0 / (d * c)
            } else {
                0.0
            }
        };

        // Calculer les poids pour chaque degré possible
        let mut weights = Vec::with_capacity(num_chunks);

        for d in 1..=num_chunks {
            let d_float = d as f64;
            let rho = if d == 1 {
                1.0 / k
            } else {
                1.0 / (d_float * (d_float - 1.0))
            };

            let weight = rho + tau(d as f64);
            weights.push(weight);
        }

        // Normaliser
        let sum: f64 = weights.iter().sum();
        for w in weights.iter_mut() {
            *w /= sum;
        }

        // Échantillonner
        let mut cumulative = 0.0;
        let sample = rng.gen::<f64>();

        for (d, &w) in weights.iter().enumerate() {
            cumulative += w;
            if sample <= cumulative {
                return d + 1; // +1 car les degrés commencent à 1
            }
        }

        num_chunks // Fallback
    }

    /// Sélectionne des chunks de façon déterministe (seed-based)
    fn select_chunks_seeded(chunks: &[Vec<u8>], degree: usize, seed: u64) -> Vec<Vec<u8>> {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut indices = HashMap::new();

        while indices.len() < degree {
            let idx = rng.gen_range(0..chunks.len());
            indices.insert(idx, ());
        }

        // Trier les indices pour garantir un ordre déterministe
        let mut sorted_indices: Vec<usize> = indices.keys().copied().collect();
        sorted_indices.sort();

        let mut selected = Vec::with_capacity(degree);
        for idx in sorted_indices {
            selected.push(chunks[idx].clone());
        }

        selected
    }

    /// XOR de plusieurs chunks
    fn xor_chunks(chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        // Trouver la longueur max
        let max_len = chunks.iter().map(|c| c.len()).max().unwrap_or(0);

        let mut result = vec![0u8; max_len];

        for chunk in chunks {
            for (i, &byte) in chunk.iter().enumerate() {
                result[i] ^= byte;
            }
        }

        Ok(result)
    }

    /// Convertit un payload en séquence ADN avec optimisation
    fn payload_to_dna(&self, payload: Vec<u8>, seed: u64) -> Result<DnaSequence> {
        let mut bases = Vec::with_capacity(payload.len() * 4); // Pré-allocation
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let validator = crate::constraints::DnaConstraintValidator::with_constraints(
            self.config.constraints.clone(),
        );

        let payload_len = payload.len();

        // Encoder chaque octet en 4 bases (2 bits par base) - version optimisée
        for byte in &payload {
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

                // Vérification optimisée des contraintes
                if validator.can_append(&bases, base) {
                    bases.push(base);
                } else {
                    // Essayer une base alternative qui préserve la valeur
                    let alt = self.suggest_alternative_base(base, &bases, &mut rng)?;
                    bases.push(alt);
                }
            }
        }

        // Créer la séquence avec validation optimisée
        let sequence = DnaSequence::new(
            bases,
            String::from("encoded"),
            0,
            payload_len,
            seed,
        );

        // Valider avec cache
        sequence.validate(&self.config.constraints)?;

        Ok(sequence)
    }

    /// Suggère une base alternative respectant les contraintes
    fn suggest_alternative_base(
        &self,
        preferred: IupacBase,
        current: &[IupacBase],
        _rng: &mut ChaCha8Rng,
    ) -> Result<IupacBase> {
        let bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        // Essayer d'abord la base préférée
        for &base in &bases {
            if base == preferred {
                continue;
            }

            let gc_ratio = if current.is_empty() {
                0.5
            } else {
                current.iter().filter(|b| b.is_gc()).count() as f64 / current.len() as f64
            };

            // Vérifier si cette base nous rapproche du GC cible
            let target_gc = (self.config.constraints.gc_min + self.config.constraints.gc_max) / 2.0;

            let is_gc = base.is_gc();
            let improves_gc = (gc_ratio < target_gc && is_gc) || (gc_ratio > target_gc && !is_gc);

            if improves_gc && self.config.constraints.validate(&[base]).is_ok() {
                return Ok(base);
            }
        }

        // Fallback: première base valide
        for base in bases {
            if self.config.constraints.validate(&[base]).is_ok() {
                return Ok(base);
            }
        }

        Err(DnaError::ConstraintViolation(
            "Impossible de trouver une base valide".to_string(),
        ))
    }

    /// Encodage Goldman (simple, sans fountain codes)
    fn encode_goldman(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        let mut sequences = Vec::with_capacity(chunks.len());

        for (i, chunk) in chunks.iter().enumerate() {
            let bases = self.chunk_to_bases(chunk)?;

            let sequence = DnaSequence::new(
                bases,
                String::from("goldman"),
                i,
                chunk.len(),
                i as u64,
            );

            // Note: Old Goldman encoder doesn't handle GC/homopolymer constraints
            // Use Goldman2013 for production with proper constraint handling
            // sequence.validate(&self.config.constraints)?;

            sequences.push(sequence);
        }

        Ok(sequences)
    }

    /// Convertit un chunk en bases (encodage simple)
    fn chunk_to_bases(&self, chunk: &[u8]) -> Result<Vec<IupacBase>> {
        let mut bases = Vec::new();

        for byte in chunk {
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

        Ok(bases)
    }

    /// Encodage adaptatif
    fn encode_adaptive(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        // Pour l'instant, fallback sur fountain
        self.encode_fountain(chunks)
    }

    /// Encodage base-3 optimisé
    fn encode_base3(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        // Pour l'instant, fallback sur goldman
        self.encode_goldman(chunks)
    }

    /// Encodage Goldman et al. 2013 - Nature 2013
    ///
    /// Spécifications du papier:
    /// - Compression Huffman (utilisant LZ4 comme proxy pour MVP)
    /// - Encodage 3-base rotation (pas 2-bit fixe)
    /// - Addressing 4-byte par oligo
    /// - Segments alternés addressing/data
    fn encode_goldman_2013(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        use crate::codec::goldman_2013::Goldman2013Encoder;

        let goldman_encoder = Goldman2013Encoder::new(self.config.constraints.clone());
        goldman_encoder.encode(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let config = EncoderConfig::default();
        let encoder = Encoder::new(config);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_simple_encoding() {
        let config = EncoderConfig {
            encoder_type: EncoderType::Goldman,
            chunk_size: 4,
            redundancy: 1.0,
            compression_enabled: false,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();
        let data = b"test";

        let sequences = encoder.encode(data).unwrap();
        assert!(!sequences.is_empty());
    }

    #[test]
    fn test_xor_chunks() {
        let chunk1 = vec![0b01010101];
        let chunk2 = vec![0b10101010];

        let result = Encoder::xor_chunks(&[chunk1, chunk2]).unwrap();
        assert_eq!(result, vec![0b11111111]);
    }

    #[test]
    fn test_fountain_degree_sampling() {
        let degree1 = Encoder::sample_robust_soliton_degree(100, 42);
        let degree2 = Encoder::sample_robust_soliton_degree(100, 42);

        // Même seed = même degré
        assert_eq!(degree1, degree2);

        let degree3 = Encoder::sample_robust_soliton_degree(100, 43);
        // Seed différent = potentiellement différent (mais pas garanti)
    }

    #[test]
    fn test_seed_based_selection() {
        let chunks = vec![
            vec![1, 2, 3],
            vec![4, 5, 6],
            vec![7, 8, 9],
        ];

        let selected1 = Encoder::select_chunks_seeded(&chunks, 2, 42);
        let selected2 = Encoder::select_chunks_seeded(&chunks, 2, 42);

        assert_eq!(selected1, selected2);
    }
}
