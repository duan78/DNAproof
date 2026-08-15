//! Décodeur ADN - Récupère les données depuis les séquences ADN

use crate::codec::fountain;
use crate::error::{DnaError, Result};
use crate::sequence::{DnaSequence, IupacBase};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

// Importer les macros depuis la racine du crate
pub use crate::{log_error, log_operation};

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
            let file = File::open(fasta_path).map_err(|e| {
                DnaError::Decoding(format!("Impossible d'ouvrir {}: {}", fasta_path, e))
            })?;

            let reader = BufReader::new(file);
            let mut sequences = Vec::new();

            // Lire le fichier et détecter le schéma
            let mut detected_scheme = None;
            let mut current_fasta = String::new();

            for line in reader.lines() {
                let line =
                    line.map_err(|e| DnaError::Decoding(format!("Erreur lecture: {}", e)))?;

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
        // Le schéma peut inclure le nombre de chunks, la longueur et la méthode :
        // "fountain#4" (ancien) ou "fountain#4#1024#l4" (format actuel)
        let scheme_raw = scheme.as_deref().unwrap_or("unknown");
        let (scheme, embedded_num_chunks, embedded_original_len, embedded_method) =
            Self::parse_scheme_suffix(scheme_raw);

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
            "ultimate" => {
                use crate::codec::ultimate::UltimateDecoder;
                let decoder = UltimateDecoder::new(crate::sequence::DnaConstraints::default());
                decoder.decode(sequences)
            }
            "fountain" | "erlich_zielinski_2017" | "adaptive" => {
                // Fountain, EZ 2017 et Adaptive partagent le même format de
                // payload (LT code + mapping 2-bits rotatif) : on délègue au
                // décodeur Fountain (peeling decoder, cf. Erlich & Zielinski 2017).
                self.decode_fountain_with_chunks(
                    sequences,
                    embedded_num_chunks,
                    embedded_original_len,
                    embedded_method,
                )
            }
            "adaptive_auto" => {
                use crate::codec::adaptive::AdaptiveDecoder;
                let decoder = AdaptiveDecoder::new(crate::sequence::DnaConstraints::default());
                decoder.decode_auto(sequences, scheme_raw)
            }
            "gc_aware" | "enhanced_gc_aware" => self.decode_gc_aware_payloads(scheme, sequences),
            "unknown" => {
                // Utiliser le décodeur générique pour inconnu
                self.decode(sequences)
            }
            _ => {
                // Schéma non reconnu, tenter le décodage générique
                self.decode(sequences)
            }
        }
    }

    /// Décode des chunks GC-aware (schémas `gc_aware` / `enhanced_gc_aware`).
    ///
    /// Chaque séquence porte un chunk de payload encodé GC-aware ; les chunks
    /// sont ordonnés par seed croissant (l'ordre d'émission des encodeurs) puis
    /// concaténés.
    fn decode_gc_aware_payloads(&self, scheme: &str, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        use crate::codec::enhanced_gc_aware::EnhancedGcAwareDecoder;
        use crate::codec::gc_aware_encoding::GcAwareDecoder;

        let constraints = crate::sequence::DnaConstraints::default();
        let mut sorted_seqs: Vec<&DnaSequence> = sequences.iter().collect();
        sorted_seqs.sort_by_key(|s| s.metadata.seed);

        let mut data = Vec::new();
        for seq in sorted_seqs {
            let chunk = if scheme == "gc_aware" {
                GcAwareDecoder::new(constraints.clone()).decode(seq)?
            } else {
                EnhancedGcAwareDecoder::new(constraints.clone()).decode(seq)?
            };
            data.extend_from_slice(&chunk);
        }
        Ok(data)
    }

    /// Decode Fountain-encoded sequences using LT code belief propagation.
    ///
    /// `known_num_chunks`: si fourni (encodé dans le schéma par l'encodeur),
    /// le peeling utilise directement cette valeur — éliminant toute ambiguïté
    /// sur le nombre de chunks. Sinon, on devine en essayant plusieurs valeurs.
    /// Parse un schéma potentiellement suffixé.
    ///
    /// Formats acceptés (les champs numériques et le marqueur de méthode sont
    /// reconnus à n'importe quelle position après le nom) :
    /// - `"fountain"` (très ancien)
    /// - `"fountain#4"` → num_chunks = 4
    /// - `"fountain#4#1024"` → + longueur du stream encodé (après chunking)
    /// - `"fountain#4#1024#l4"` → + méthode de compression (`l4`/`zst`/`raw`)
    /// - `"goldman#raw"` (schémas génériques)
    fn parse_scheme_suffix(
        scheme_full: &str,
    ) -> (&str, Option<usize>, Option<usize>, Option<&str>) {
        let mut parts = scheme_full.split('#');
        let scheme = parts.next().unwrap_or("");
        let mut num_chunks = None;
        let mut original_len = None;
        let mut method = None;

        for part in parts {
            match part {
                "l4" | "zst" | "raw" => method = Some(part),
                _ => {
                    if let Ok(n) = part.parse::<usize>() {
                        if num_chunks.is_none() {
                            num_chunks = Some(n);
                        } else if original_len.is_none() {
                            original_len = Some(n);
                        }
                    }
                }
            }
        }

        (scheme, num_chunks, original_len, method)
    }

    /// Décompresse un stream selon le marqueur de méthode encodé par l'encodeur.
    /// `raw` retourne les données telles quelles.
    fn decompress_by_marker(&self, data: &[u8], method: &str) -> Result<Vec<u8>> {
        match method {
            "raw" => Ok(data.to_vec()),
            "l4" => lz4::block::decompress(data, None)
                .map_err(|e| DnaError::Decoding(format!("Erreur décompression LZ4: {}", e))),
            "zst" => zstd::decode_all(data)
                .map_err(|e| DnaError::Decoding(format!("Erreur décompression Zstd: {}", e))),
            other => Err(DnaError::Decoding(format!(
                "Marqueur de compression inconnu: {}",
                other
            ))),
        }
    }

    fn decode_fountain_with_chunks(
        &self,
        sequences: &[DnaSequence],
        known_num_chunks: Option<usize>,
        known_original_len: Option<usize>,
        known_method: Option<&str>,
    ) -> Result<Vec<u8>> {
        if sequences.is_empty() {
            return Err(DnaError::Decoding("Aucune séquence fournie".to_string()));
        }

        // Step 1: Extract payloads from sequences
        let mut droplet_data: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut chunk_size = 0;

        for seq in sequences {
            // Decode DNA to bytes
            let payload = self.sequence_to_chunk(seq)?;
            if chunk_size == 0 {
                chunk_size = payload.len();
            }
            droplet_data.push((seq.metadata.seed, payload));
        }

        // Step 2: Determine the number of chunks.
        // Si l'encodeur a stocké num_chunks dans le schéma, on l'utilise
        // directement — c'est fiable et sans ambiguïté.
        // Sinon, on devine en essayant plusieurs valeurs (fallback legacy).
        let num_droplets = droplet_data.len();
        let max_seed = droplet_data.iter().map(|(s, _)| *s).max().unwrap_or(0) as usize;
        let estimated_max_chunks = (max_seed + 1).max(num_droplets);

        // Construire la liste des num_chunks à essayer : priorité au num_chunks connu
        let guesses: Vec<usize> = match known_num_chunks {
            Some(n) if n >= 1 => vec![n],
            _ => (1..=estimated_max_chunks).collect(),
        };

        let mut best_result: Option<Vec<u8>> = None;

        for num_chunks_guess in guesses {
            let mut decoder =
                FountainDecoder::new(self.config.clone(), num_chunks_guess, chunk_size);

            for (seed, payload) in &droplet_data {
                let degree = fountain::sample_robust_soliton_degree(num_chunks_guess, *seed);
                let chunk_indices = fountain::select_chunk_indices(num_chunks_guess, degree, *seed);
                let droplet = Droplet::new(chunk_indices, payload.clone(), *seed);

                match decoder.add_droplet(droplet)? {
                    Progress::Complete(data) => {
                        // Retirer le padding du dernier chunk si la longueur du
                        // stream encodé est connue (schéma récent).
                        let stream = match known_original_len {
                            Some(len) if len <= data.len() => data[..len].to_vec(),
                            _ => data.clone(),
                        };

                        match known_method {
                            // Méthode explicite encodée par l'encodeur : pas
                            // d'heuristique, pas de risque de décompression
                            // spurieuse sur des données non compressées.
                            Some(method) => {
                                if self.config.auto_decompress {
                                    return self.decompress_by_marker(&stream, method);
                                }
                                // auto_decompress=false : l'appelant veut le
                                // stream tel quel (déjà tronqué du padding).
                                return Ok(stream);
                            }
                            // Ancien format sans marqueur : heuristiques legacy.
                            None => {
                                if self.config.auto_decompress
                                    && self.config.compression_type != CompressionType::None
                                {
                                    // Le peeling peut produire un buffer plus grand que le
                                    // stream compressé (padding de chunks en fin). LZ4/zstd
                                    // rejettent les bytes trailing, donc on essaie de tronquer
                                    // progressivement depuis la fin jusqu'à trouver la taille
                                    // exacte du stream compressé.
                                    if let Some(decompressed) =
                                        self.try_decompress_with_padding(&data, None)
                                    {
                                        return Ok(decompressed);
                                    }
                                    // La décompression a échoué : essaie le prochain num_chunks
                                } else if best_result.is_none() {
                                    best_result = Some(stream);
                                }
                            }
                        }
                        break;
                    }
                    Progress::Incomplete => continue,
                }
            }
        }

        if let Some(data) = best_result {
            return Ok(data);
        }

        Err(DnaError::Decoding(
            "Fountain decoding failed: insufficient droplets or incorrect chunk count estimation"
                .to_string(),
        ))
    }

    /// Décode des séquences ADN en données avec gestion des erreurs améliorée.
    ///
    /// Route automatiquement vers le bon décodeur selon le schéma d'encodage
    /// détecté dans les métadonnées des séquences (`encoding_scheme`).
    /// Pour les schémas Fountain / EZ 2017, utilise le peeling decoder LT ;
    /// sinon, fait un décodage générique (tri par chunk_index + concaténation).
    pub fn decode(&self, sequences: &[DnaSequence]) -> Result<Vec<u8>> {
        log_operation!("decode_data", {
            if sequences.is_empty() {
                return Err(DnaError::Decoding("Aucune séquence fournie".to_string()));
            }

            // Détecter le schéma d'encodage depuis les métadonnées de la 1ère séquence.
            // Le schéma peut inclure le nombre de chunks, la longueur et la méthode :
            // "fountain#4#1024#l4", "goldman#raw", etc.
            let scheme_full = sequences[0].metadata.encoding_scheme.as_str();
            let (scheme, embedded_num_chunks, embedded_original_len, embedded_method) =
                Self::parse_scheme_suffix(scheme_full);

            // Router vers le décodeur spécialisé si applicable
            match scheme {
                "fountain" | "erlich_zielinski_2017" | "adaptive" => {
                    return self.decode_fountain_with_chunks(
                        sequences,
                        embedded_num_chunks,
                        embedded_original_len,
                        embedded_method,
                    );
                }
                "goldman_2013" => {
                    use crate::codec::goldman_2013::Goldman2013Decoder;
                    let decoder =
                        Goldman2013Decoder::new(crate::sequence::DnaConstraints::default());
                    return decoder.decode(sequences);
                }
                "grass_2015" => {
                    use crate::codec::grass_2015::Grass2015Decoder;
                    let decoder = Grass2015Decoder::new(crate::sequence::DnaConstraints::default());
                    return decoder.decode(sequences);
                }
                "ultimate" => {
                    use crate::codec::ultimate::UltimateDecoder;
                    let decoder = UltimateDecoder::new(crate::sequence::DnaConstraints::default());
                    return decoder.decode(sequences);
                }
                "adaptive_auto" => {
                    use crate::codec::adaptive::AdaptiveDecoder;
                    let decoder = AdaptiveDecoder::new(crate::sequence::DnaConstraints::default());
                    return decoder.decode_auto(sequences, scheme_full);
                }
                "gc_aware" | "enhanced_gc_aware" => {
                    return self.decode_gc_aware_payloads(scheme, sequences);
                }
                _ => {} // Schéma inconnu : décodage générique ci-dessous
            }

            // Décodage générique (Goldman-like) : tri par chunk_index + concaténation
            let mut data = Vec::new();

            // Trier les séquences par chunk_index
            let mut sorted_seqs: Vec<_> = sequences.iter().collect();
            sorted_seqs.sort_by_key(|s| s.metadata.chunk_index);

            for seq in sorted_seqs {
                let chunk_data = self.sequence_to_chunk(seq)?;
                data.extend_from_slice(&chunk_data);
            }

            // Décompression : le marqueur encodé par l'encodeur (schéma récent)
            // est déterministe ; sans marqueur, heuristique Auto legacy.
            let result = if self.config.auto_decompress {
                match embedded_method {
                    Some(method) => self.decompress_by_marker(&data, method)?,
                    None => self.decompress(&data)?,
                }
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

    /// Convertit une séquence en chunk de données.
    ///
    /// Détecte le schéma d'encodage depuis les métadonnées pour savoir si
    /// l'encodage rotatif a été utilisé (Fountain/EZ2017) ou non (Goldman legacy).
    fn sequence_to_chunk(&self, sequence: &DnaSequence) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let bases = &sequence.bases;

        // Déterminer si l'encodage rotatif a été utilisé selon le schéma.
        // Le schéma peut contenir un suffixe "#N" (num_chunks) qu'on ignore ici.
        let scheme = sequence
            .metadata
            .encoding_scheme
            .split('#')
            .next()
            .unwrap_or("");
        let use_rotation = matches!(scheme, "fountain" | "erlich_zielinski_2017" | "adaptive");

        // Tables de rotation inverses
        fn base_to_bits_rotated(base: IupacBase, rotation: usize) -> Result<u8> {
            let bits = match (base, rotation % 4) {
                (IupacBase::A, 0) | (IupacBase::C, 1) | (IupacBase::G, 2) | (IupacBase::T, 3) => {
                    0b00
                }
                (IupacBase::C, 0) | (IupacBase::G, 1) | (IupacBase::T, 2) | (IupacBase::A, 3) => {
                    0b01
                }
                (IupacBase::G, 0) | (IupacBase::T, 1) | (IupacBase::A, 2) | (IupacBase::C, 3) => {
                    0b10
                }
                (IupacBase::T, 0) | (IupacBase::A, 1) | (IupacBase::C, 2) | (IupacBase::G, 3) => {
                    0b11
                }
                _ => {
                    return Err(DnaError::Decoding(format!(
                        "Base non-standard décodée: {:?}",
                        base
                    )))
                }
            };
            Ok(bits)
        }

        // Décodage : 4 bases = 1 octet (2 bits par base)
        for (global_idx, base) in bases.iter().enumerate() {
            let bits = if use_rotation {
                base_to_bits_rotated(*base, global_idx)?
            } else {
                // Mapping fixe (sans rotation) pour Goldman legacy
                match base {
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
                }
            };

            let bit_in_byte = global_idx % 4;
            let bit_offset = 6 - 2 * bit_in_byte;

            if bit_in_byte == 0 {
                data.push(0u8);
            }
            let last = data.last_mut().unwrap();
            *last |= bits << bit_offset;
        }

        Ok(data)
    }

    /// Tente de décompresser des données qui peuvent contenir du padding en fin
    /// de buffer (issu du peeling decoder Fountain avec chunks padés).
    ///
    /// LZ4/zstd rejettent les bytes trailing, donc on essaie de décompresser
    /// le buffer complet d'abord, puis en tronquant progressivement depuis la fin
    /// jusqu'à trouver la taille exacte du stream compressé.
    ///
    /// `expected_len` (longueur des données originales avant compression, quand
    /// l'encodeur l'a encodée dans le schéma) sert de validation : une
    /// "décompression réussie" de taille différente est rejetée — cela évite
    /// qu'un stream LZ4 spurieux soit accepté sur des données non compressées.
    /// Retourne None si aucune taille ne permet de décompresser.
    fn try_decompress_with_padding(
        &self,
        data: &[u8],
        expected_len: Option<usize>,
    ) -> Option<Vec<u8>> {
        // Essaie de décompresser en ignorant le fallback "None" de Auto.
        // On teste LZ4 et zstd directement, car le fallback None retournerait
        // les données brutes (y compris le padding) sans validation.
        let try_one = |buf: &[u8]| -> Option<Vec<u8>> {
            if let Ok(d) = lz4::block::decompress(buf, None) {
                if !d.is_empty() && expected_len.is_none_or(|l| d.len() == l) {
                    return Some(d);
                }
            }
            if let Ok(d) = zstd::decode_all(buf) {
                if !d.is_empty() && expected_len.is_none_or(|l| d.len() == l) {
                    return Some(d);
                }
            }
            None
        };

        // D'abord essayer la taille complète (pas de padding)
        if let Some(d) = try_one(data) {
            return Some(d);
        }
        // Tronquer depuis la fin pour trouver la taille exacte du stream compressé.
        // Le préfixe LZ4 (compress_prepend_size) occupe 4 bytes, donc minimum 5 bytes.
        for trim in 1..data.len() {
            let truncated = &data[..data.len() - trim];
            if truncated.len() < 5 {
                break;
            }
            if let Some(d) = try_one(truncated) {
                return Some(d);
            }
        }
        None
    }

    /// Décompresse les données
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        let compression_type = match self.config.compression_type {
            CompressionType::Auto => {
                // Auto-détection: essayer LZ4 (lit le préfixe taille si présent) puis Zstd
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
                // decompress(data, None) lit le préfixe de taille écrit par compress(_, _, true)
                lz4::block::decompress(data, None)
                    .map_err(|e| DnaError::Decoding(format!("Erreur décompression LZ4: {}", e)))
            }
            CompressionType::Zstd => zstd::decode_all(data)
                .map_err(|e| DnaError::Decoding(format!("Erreur décompression Zstd: {}", e))),
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
        let max_iterations = self.config.max_iterations;

        for _iteration in 0..max_iterations {
            // Trouver les droplets de degré 1 dont le chunk n'est pas encore extrait
            let degree_one_droplets = self.find_degree_one_droplets();

            if degree_one_droplets.is_empty() {
                // Plus aucun droplet de degré 1 exploitable
                if self.chunks.len() == self.required {
                    return Ok(Progress::Complete(self.reassemble()?));
                }
                return Ok(Progress::Incomplete);
            }

            let chunks_before = self.chunks.len();

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

            // Si aucun nouveau chunk n'a été extrait ce tour, on est bloqué
            if self.chunks.len() == chunks_before {
                return Ok(Progress::Incomplete);
            }
        }

        Err(DnaError::Decoding(
            "Belief propagation: nombre maximum d'itérations atteint".to_string(),
        ))
    }

    /// Trouve tous les indices des droplets de degré 1
    fn find_degree_one_droplets(&self) -> Vec<usize> {
        self.droplets
            .iter()
            .enumerate()
            .filter(|(_, d)| d.degree() == 1)
            .map(|(i, _)| i)
            .collect()
    }

    /// XOR un chunk de tous les droplets qui le contiennent
    fn xor_out_chunk(&mut self, chunk_idx: usize, chunk_data: &[u8]) {
        for droplet in &mut self.droplets {
            // Chercher ce chunk dans les indices du droplet
            if let Some(pos) = droplet
                .chunk_indices
                .iter()
                .position(|&idx| idx == chunk_idx)
            {
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

        assert!(matches!(
            decoder.add_droplet(droplet1).unwrap(),
            Progress::Incomplete
        ));
        assert!(matches!(
            decoder.add_droplet(droplet2).unwrap(),
            Progress::Incomplete
        ));

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
        assert!(matches!(
            decoder.add_droplet(droplet2).unwrap(),
            Progress::Incomplete
        ));
        assert!(matches!(
            decoder.add_droplet(droplet1).unwrap(),
            Progress::Incomplete
        ));

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

        assert!(matches!(
            decoder.add_droplet(droplet1).unwrap(),
            Progress::Incomplete
        ));
        assert!(matches!(
            decoder.add_droplet(droplet2).unwrap(),
            Progress::Incomplete
        ));
        assert!(matches!(
            decoder.add_droplet(droplet3).unwrap(),
            Progress::Incomplete
        ));

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
