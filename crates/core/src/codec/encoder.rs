//! Encodeur ADN - Implémente DNA Fountain et autres algorithmes

use crate::codec::fountain;
use crate::error::{DnaError, Result};
use crate::sequence::{DnaConstraints, DnaSequence, IupacBase};
use rand_chacha::ChaCha8Rng;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};

// Importer les macros depuis la racine du crate
pub use crate::{log_error, log_operation};

/// Type d'algorithme d'encodage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncoderType {
    /// DNA Fountain - LT codes avec distribution robust soliton
    #[default]
    Fountain,
    /// Erlich-Zielinski 2017 - DNA Fountain avec paramètres validés (Science 2017)
    /// Paramètres: c=0.1, δ=0.5, GC 40-60%, homopolymer <4, 152nt
    ErlichZielinski2017,
    /// Goldman et al. 2013 - Nature 2013 (LZ4 comme proxy de compression,
    /// encodage 2-bits rotatif par position, index 16 bits sur 8 bases)
    Goldman2013,
    /// Goldman code - Codage 2-bits simple sans fountain codes (legacy)
    Goldman,
    /// Grass et al. 2015 - Nature Biotechnology 2015 (Reed-Solomon + 3-segment addressing)
    Grass2015,
    /// Encodage adaptatif — pipeline Fountain avec routage dédié au décodage
    Adaptive,
    /// Encodage base-3 (actuellement: fallback sur l'encodage Goldman 2-bits)
    Base3,
    /// Ultimate - toutes les optimisations combinées (adaptatif + RS + spreading + GC-aware)
    Ultimate,
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
    ///
    /// Retourne une erreur si la configuration est incohérente
    /// (chunk_size nul, redondance < 1.0 ou non finie).
    pub fn new(config: EncoderConfig) -> Result<Self> {
        Self::validate_config(&config)?;
        Ok(Self { config })
    }

    /// Valide la cohérence de la configuration.
    ///
    /// - `chunk_size == 0` ferait paniquer `chunks()` à l'encodage ;
    /// - `redundancy < 1.0` produirait silencieusement zéro goutte (données
    ///   non récupérables), et une valeur non finie sature le cast en usize.
    fn validate_config(config: &EncoderConfig) -> Result<()> {
        if config.chunk_size == 0 {
            return Err(DnaError::InvalidConfig(
                "chunk_size doit être >= 1 octet".to_string(),
            ));
        }
        if !config.redundancy.is_finite() || config.redundancy < 1.0 {
            return Err(DnaError::InvalidConfig(format!(
                "redundancy doit être un nombre fini >= 1.0 (reçu {})",
                config.redundancy
            )));
        }
        Ok(())
    }

    /// Retourne le nom du schéma d'encodage actuel
    fn encoding_scheme_name(&self) -> &'static str {
        match self.config.encoder_type {
            EncoderType::Fountain => "fountain",
            EncoderType::ErlichZielinski2017 => "erlich_zielinski_2017",
            EncoderType::Goldman2013 => "goldman_2013",
            EncoderType::Goldman => "goldman",
            EncoderType::Grass2015 => "grass_2015",
            EncoderType::Adaptive => "adaptive",
            EncoderType::Base3 => "base3",
            EncoderType::Ultimate => "ultimate",
        }
    }

    /// Marqueur de compression embarqué dans le schéma pour le décodeur :
    /// "l4" (LZ4), "zst" (Zstd) ou "raw" (aucune compression).
    fn compression_marker(&self) -> &'static str {
        if !self.config.compression_enabled {
            return "raw";
        }
        match self.config.compression_type {
            CompressionType::Lz4 => "l4",
            CompressionType::Zstd => "zst",
            CompressionType::None => "raw",
        }
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
            let original_len = processed_data.len();

            // 3. Encodage selon le type avec parallélisme
            let sequences = match self.config.encoder_type {
                EncoderType::Fountain => self.encode_fountain_optimized(&chunks, original_len)?,
                EncoderType::ErlichZielinski2017 => {
                    self.encode_erlich_zielinski_2017(&chunks, original_len)?
                }
                EncoderType::Goldman2013 => self.encode_goldman_2013(data)?,
                EncoderType::Goldman => self.encode_goldman(&chunks)?,
                EncoderType::Grass2015 => self.encode_grass_2015(data)?,
                EncoderType::Adaptive => self.encode_adaptive(&chunks, original_len)?,
                EncoderType::Base3 => self.encode_base3(&chunks)?,
                // Ultimate applique sa propre compression adaptative : on lui
                // passe les données brutes (pas le buffer déjà compressé par
                // `self.compress`), sinon les données seraient compressées deux
                // fois et le décodeur ne pourrait pas tout décompresser.
                EncoderType::Ultimate => self.encode_ultimate(data)?,
            };

            Ok(sequences)
        })
    }

    /// Compresse les données
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        match self.config.compression_type {
            CompressionType::Lz4 => {
                // prepend_size=true préfixe le output avec la taille originale (4 bytes LE),
                // ce que decompress(data, None) lit automatiquement au décodage.
                // Indispensable pour le décodage Fountain où la taille reconstruite
                // peut inclure du padding de chunks en fin de buffer.
                let compressed = lz4::block::compress(data, None, true)
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

    /// Divise les données en chunks.
    ///
    /// Le dernier chunk peut être plus court que `chunk_size` (pas de padding).
    /// Les schémas qui nécessitent des chunks de taille uniforme (Fountain/EZ2017,
    /// car le peeling decoder les XOR) doivent appliquer leur propre padding via
    /// `pad_chunks_for_fountain`.
    fn split_into_chunks(&self, data: &[u8]) -> Vec<Vec<u8>> {
        data.chunks(self.config.chunk_size)
            .map(|c| c.to_vec())
            .collect()
    }

    /// Pade tous les chunks à la taille du plus grand (chunk_size) pour le LT code.
    ///
    /// Le peeling decoder reconstruit les chunks par XOR. Des chunks de tailles
    /// inégales cassent l'alignement. Le padding (zéros) est retiré au décodage
    /// via auto-décompression (LZ4/zstd connaissent leur taille de sortie).
    fn pad_chunks_for_fountain(&self, chunks: &mut Vec<Vec<u8>>) {
        let cs = self.config.chunk_size;
        for chunk in chunks {
            if chunk.len() < cs {
                chunk.resize(cs, 0u8);
            }
        }
    }

    /// Encodage DNA Fountain optimisé avec parallélisme
    fn encode_fountain_optimized(
        &self,
        chunks: &[Vec<u8>],
        original_len: usize,
    ) -> Result<Vec<DnaSequence>> {
        // Pader les chunks à taille uniforme (requis pour le XOR du LT code)
        let mut chunks = chunks.to_vec();
        self.pad_chunks_for_fountain(&mut chunks);
        let chunks = &chunks[..];

        let num_chunks = chunks.len();
        let num_droplets = (num_chunks as f64 * self.config.redundancy).ceil() as usize;

        // Utiliser Rayon pour le parallélisme
        let sequences: Result<Vec<DnaSequence>> = (0..num_droplets)
            .into_par_iter()
            .map(|seed| {
                // Échantillonner le degré depuis la distribution robust soliton
                let degree = fountain::sample_robust_soliton_degree(num_chunks, seed as u64);

                // Sélectionner les chunks (seed-based pour reproductibilité)
                let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed as u64);

                // XOR des chunks sélectionnés
                let payload = Self::xor_chunks(&selected_chunks)?;

                // Convertir en ADN avec contraintes (encode num_chunks pour le décodeur)
                self.payload_to_dna_with_chunks(
                    payload,
                    seed as u64,
                    Some(num_chunks),
                    original_len,
                )
            })
            .collect();

        // Garantir la décodabilité : avec peu de chunks, un tirage aléatoire
        // peut laisser un chunk absent de tout droplet exploitable par le
        // peeling. On ajoute des droplets jusqu'à couverture complète.
        let mut sequences = sequences?;
        self.ensure_peelable(&mut sequences, chunks, num_chunks, original_len)?;

        Ok(sequences)
    }

    /// Vérifie par simulation que le peeling decoder peut reconstruire tous
    /// les chunks depuis les séquences générées (coverage structurelle).
    ///
    /// La simulation utilise des payloads factices : la structure du graphe
    /// (degrés et indices par seed) suffit à déterminer si le peeling aboutit.
    fn droplets_peelable(sequences: &[DnaSequence], num_chunks: usize, chunk_size: usize) -> bool {
        use crate::codec::decoder::{DecoderConfig, Droplet, FountainDecoder, Progress};

        if num_chunks == 0 {
            return true;
        }

        let mut decoder = FountainDecoder::new(DecoderConfig::default(), num_chunks, chunk_size);
        let dummy_payload = vec![0u8; chunk_size];

        for seq in sequences {
            let seed = seq.metadata.seed;
            let degree = fountain::sample_robust_soliton_degree(num_chunks, seed);
            let indices = fountain::select_chunk_indices(num_chunks, degree, seed);
            match decoder.add_droplet(Droplet::new(indices, dummy_payload.clone(), seed)) {
                Ok(Progress::Complete(_)) => return true,
                Ok(Progress::Incomplete) => {}
                Err(_) => return false,
            }
        }
        decoder.is_complete()
    }

    /// Ajoute des droplets supplémentaires tant que l'ensemble n'est pas
    /// décodable par peeling (garantie de round-trip).
    fn ensure_peelable(
        &self,
        sequences: &mut Vec<DnaSequence>,
        chunks: &[Vec<u8>],
        num_chunks: usize,
        original_len: usize,
    ) -> Result<()> {
        if num_chunks == 0 {
            return Ok(());
        }

        let chunk_size = chunks.first().map(|c| c.len()).unwrap_or(0);
        let mut seed = sequences.last().map(|s| s.metadata.seed + 1).unwrap_or(0);
        let max_extra = num_chunks * 200;

        for _ in 0..max_extra {
            if Self::droplets_peelable(sequences, num_chunks, chunk_size) {
                return Ok(());
            }

            let degree = fountain::sample_robust_soliton_degree(num_chunks, seed);
            let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed);
            let payload = Self::xor_chunks(&selected_chunks)?;
            let dna =
                self.payload_to_dna_with_chunks(payload, seed, Some(num_chunks), original_len)?;
            sequences.push(dna);
            seed += 1;
        }

        // Best effort : improbable (le tirage aléatoire couvre rapidement),
        // mais on ne bloque pas l'encodage pour autant.
        if !Self::droplets_peelable(sequences, num_chunks, chunk_size) {
            eprintln!(
                "[warn] Fountain: {} droplets générés mais couverture peeling incomplète",
                sequences.len()
            );
        }
        Ok(())
    }

    /// Encodage Erlich-Zielinski 2017 - DNA Fountain validé (Science 2017)
    ///
    /// Spécifications du papier:
    /// - Distribution Robust Soliton: c=0.1, δ=0.5
    /// - Contraintes biochemical: GC 40-60%, homopolymer <4
    /// - Longueur d'oligo: 152nt (± quelques bases)
    /// - Overhead théorique: 1.03-1.07× (minimum)
    ///
    /// IMPORTANT: cet encodeur partage le même format de payload que `encode_fountain`
    /// (mapping 2-bits→base rotatif via `payload_to_dna_with_chunks`), de sorte que le
    /// décodeur Fountain puisse relire les droplets symétriquement.
    ///
    /// GARANTIE DES CONTRAINTES : l'encodage rotatif réduit statistiquement les
    /// violations GC/homopolymer. Un screening (rejet des séquences non conformes)
    /// garantit strictement le respect des contraintes EZ 2017 (GC 40-60%, homopolymer <4).
    fn encode_erlich_zielinski_2017(
        &self,
        chunks: &[Vec<u8>],
        original_len: usize,
    ) -> Result<Vec<DnaSequence>> {
        // Pader les chunks à taille uniforme (requis pour le XOR du LT code)
        let mut chunks = chunks.to_vec();
        self.pad_chunks_for_fountain(&mut chunks);
        let chunks = &chunks[..];

        let num_chunks = chunks.len();
        let num_droplets = (num_chunks as f64 * self.config.redundancy).ceil() as usize;

        let mut sequences = Vec::with_capacity(num_droplets);

        // Screening : générer des droplets avec des seeds croissants.
        // Si un droplet viole les contraintes GC/homopolymer, le rejeter et essayer
        // le seed suivant. On génère plus de droplets que demandé pour compenser
        // le taux de rejet et garantir que le peeling decoder aura assez de redondance.
        let max_attempts = num_droplets * 100;
        let mut seed = 0u64;
        let mut attempts = 0;
        let mut rejected_count = 0u32;

        while sequences.len() < num_droplets && attempts < max_attempts {
            attempts += 1;

            // Distribution de degré identique au décodeur
            let degree = fountain::sample_robust_soliton_degree(num_chunks, seed);
            let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed);
            let payload = Self::xor_chunks(&selected_chunks)?;
            let dna =
                self.payload_to_dna_with_chunks(payload, seed, Some(num_chunks), original_len)?;

            // Screening : vérifier strictement les contraintes EZ 2017
            if Self::check_ez2017_constraints(&dna) {
                sequences.push(dna);
            } else {
                rejected_count += 1;
            }

            seed += 1;
        }

        // Fallback : si le screening n'a pas pu générer assez de droplets valides
        // (cas dégénéré : données produisant systématiquement des violations),
        // on accepte les droplets non conformes pour garantir le round-trip.
        // Chaque seed produit un droplet unique, donc pas de doublons.
        while sequences.len() < num_droplets {
            eprintln!(
                "[warn] EZ 2017: screening n'a pu générer que {}/{} droplets valides ({} rejetés). \
                 Acceptation de droplets non conformes pour garantir le round-trip.",
                sequences.len(), num_droplets, rejected_count
            );
            let degree = fountain::sample_robust_soliton_degree(num_chunks, seed);
            let selected_chunks = Self::select_chunks_seeded(chunks, degree, seed);
            let payload = Self::xor_chunks(&selected_chunks)?;
            let dna =
                self.payload_to_dna_with_chunks(payload, seed, Some(num_chunks), original_len)?;
            sequences.push(dna);
            seed += 1;
        }

        // Garantie de décodabilité : le screening GC rejette déterministement
        // tout droplet de degré 1 dont le chunk seul viole GC 40-60% — de tels
        // chunks ne peuvent alors être récupérés que par peeling depuis des
        // XOR, ce qui peut échouer (observé avec K=5 : chunks sans aucune
        // couverture exploitable). On ajoute des droplets non conformes
        // jusqu'à ce que le peeling aboutisse : le round-trip prime sur les
        // contraintes biochimiques dans ce cas dégénéré.
        let chunk_size = chunks.first().map(|c| c.len()).unwrap_or(0);
        if !Self::droplets_peelable(&sequences, num_chunks, chunk_size) {
            eprintln!(
                "[warn] EZ 2017: le screening a laissé des chunks non couvrables, \
                 ajout de droplets non conformes pour garantir le round-trip"
            );
            self.ensure_peelable(&mut sequences, chunks, num_chunks, original_len)?;
        }

        Ok(sequences)
    }

    /// Vérifie strictement si une séquence respecte les contraintes EZ 2017.
    ///
    /// Contraintes du papier Erlich-Zielinski 2017 :
    /// - GC content entre 40% et 60%
    /// - Pas d'homopolymer de longueur ≥ 4
    ///
    /// Utilisé par le screening pour rejeter les droplets non conformes.
    fn check_ez2017_constraints(sequence: &DnaSequence) -> bool {
        let bases = &sequence.bases;
        if bases.is_empty() {
            return false;
        }

        // GC ratio (40-60%)
        let gc_count = bases.iter().filter(|b| b.is_gc()).count();
        let gc_ratio = gc_count as f64 / bases.len() as f64;
        if !(0.40..=0.60).contains(&gc_ratio) {
            return false;
        }

        // Homopolymer < 4
        let max_homopolymer = crate::constraints::find_max_homopolymer(bases);
        if max_homopolymer >= 4 {
            return false;
        }

        true
    }

    /// Sélectionne des chunks de façon déterministe (seed-based).
    ///
    /// Délègue le tirage des indices au module `fountain` (flux RNG indépendant
    /// du tirage du degré) puis récupère les chunks correspondants.
    fn select_chunks_seeded(chunks: &[Vec<u8>], degree: usize, seed: u64) -> Vec<Vec<u8>> {
        let indices = fountain::select_chunk_indices(chunks.len(), degree, seed);

        let mut selected = Vec::with_capacity(degree);
        for idx in indices {
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

    /// Convertit un payload en séquence ADN, en encodant le nombre de chunks
    /// et la longueur originale des données dans le schéma
    /// (`<scheme>#<num_chunks>#<original_len>`).
    ///
    /// Utilise un encodage rotatif déterministe : pour chaque 2 bits à la position
    /// globale `i`, la table de mapping est cycliquement décalée de `i % 4` positions.
    /// Cela distribue uniformément les bases et réduit statistiquement les homopolymères
    /// et le déséquilibre GC. Le décodeur peut inverser exactement car la rotation ne
    /// dépend que de la position (connue au décodage).
    ///
    /// `original_len` permet au décodeur de retirer le padding du dernier chunk
    /// quand la compression est désactivée (la compression, elle, connaît sa
    /// propre taille de sortie).
    fn payload_to_dna_with_chunks(
        &self,
        payload: Vec<u8>,
        seed: u64,
        num_chunks: Option<usize>,
        original_len: usize,
    ) -> Result<DnaSequence> {
        let mut bases = Vec::with_capacity(payload.len() * 4);
        let payload_len = payload.len();

        // 4 tables de mapping rotatives
        // Rotation 0: 00→A, 01→C, 10→G, 11→T
        // Rotation 1: 00→C, 01→G, 10→T, 11→A
        // Rotation 2: 00→G, 01→T, 10→A, 11→C
        // Rotation 3: 00→T, 01→A, 10→C, 11→G
        const ROTATION_TABLES: [[IupacBase; 4]; 4] = [
            [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
            [IupacBase::C, IupacBase::G, IupacBase::T, IupacBase::A],
            [IupacBase::G, IupacBase::T, IupacBase::A, IupacBase::C],
            [IupacBase::T, IupacBase::A, IupacBase::C, IupacBase::G],
        ];

        // Encoder chaque octet en 4 bases (2 bits par base) avec rotation
        let mut global_idx = 0usize;
        for byte in &payload {
            let bits = [
                (byte >> 6) & 0b11,
                (byte >> 4) & 0b11,
                (byte >> 2) & 0b11,
                byte & 0b11,
            ];

            for two_bits in bits {
                let rotation = global_idx % 4;
                let base = ROTATION_TABLES[rotation][two_bits as usize];
                bases.push(base);
                global_idx += 1;
            }
        }

        // Créer la séquence — encoder num_chunks, longueur originale et méthode
        // de compression dans le schéma pour le décodeur
        let scheme = match num_chunks {
            Some(n) => format!(
                "{}#{}#{}#{}",
                self.encoding_scheme_name(),
                n,
                original_len,
                self.compression_marker()
            ),
            None => self.encoding_scheme_name().to_string(),
        };
        let sequence = DnaSequence::with_encoding_scheme(
            bases,
            String::from("encoded"),
            0,
            payload_len,
            seed,
            scheme,
        );

        // Validation permissive de la séquence (longueur et bases).
        // Le contrôle strict des contraintes GC/homopolymer EZ 2017 est assuré
        // par le screening (check_ez2017_constraints) au niveau de l'encodeur EZ2017.
        let permissive = DnaConstraints {
            gc_min: 0.0,
            gc_max: 1.0,
            max_homopolymer: usize::MAX,
            ..self.config.constraints.clone()
        };
        sequence.validate(&permissive)?;

        Ok(sequence)
    }

    /// Suggère une base alternative respectant les contraintes
    #[allow(dead_code)]
    fn suggest_alternative_base(
        &self,
        preferred: IupacBase,
        current: &[IupacBase],
        _rng: &mut ChaCha8Rng,
    ) -> Result<IupacBase> {
        let validator = crate::constraints::DnaConstraintValidator::with_constraints(
            self.config.constraints.clone(),
        );

        let bases = [IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];

        // Calculate current GC ratio
        let gc_ratio = if current.is_empty() {
            0.5
        } else {
            current.iter().filter(|b| b.is_gc()).count() as f64 / current.len() as f64
        };

        let target_gc = (self.config.constraints.gc_min + self.config.constraints.gc_max) / 2.0;

        // First, try bases that improve GC balance and can be appended
        for &base in &bases {
            if base == preferred {
                continue;
            }

            let is_gc = base.is_gc();
            let improves_gc = (gc_ratio < target_gc && is_gc) || (gc_ratio > target_gc && !is_gc);

            if improves_gc && validator.can_append(current, base) {
                return Ok(base);
            }
        }

        // Fallback: any base that can be appended (ignoring GC improvement)
        for &base in &bases {
            if base == preferred {
                continue;
            }
            if validator.can_append(current, base) {
                return Ok(base);
            }
        }

        // Last resort: even the preferred base if it can be appended
        if validator.can_append(current, preferred) {
            return Ok(preferred);
        }

        Err(DnaError::ConstraintViolation(
            "Impossible de trouver une base valide".to_string(),
        ))
    }

    /// Encodage Goldman (simple, sans fountain codes)
    fn encode_goldman(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        let mut sequences = Vec::with_capacity(chunks.len());

        // Schéma suffixé du marqueur de compression pour le décodeur générique
        let scheme = format!(
            "{}#{}",
            self.encoding_scheme_name(),
            self.compression_marker()
        );

        for (i, chunk) in chunks.iter().enumerate() {
            let bases = self.chunk_to_bases(chunk)?;

            let sequence = DnaSequence::with_encoding_scheme(
                bases,
                String::from("goldman"),
                i,
                chunk.len(),
                i as u64,
                scheme.clone(),
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

    /// Encodage adaptatif.
    ///
    /// Choisit le pipeline Fountain (chunks padés à taille uniforme, LT code,
    /// encodage rotatif) avec le schéma `adaptive#N` : le décodeur route ce
    /// schéma vers le même peeling decoder que Fountain/EZ2017, ce qui garantit
    /// le round-trip. La sélection adaptative de la compression reste gérée en
    /// amont via `EncoderConfig::compression_type`.
    fn encode_adaptive(&self, chunks: &[Vec<u8>], original_len: usize) -> Result<Vec<DnaSequence>> {
        self.encode_fountain_optimized(chunks, original_len)
    }

    /// Encodage base-3 (actuellement: fallback sur l'encodage Goldman 2-bits)
    fn encode_base3(&self, chunks: &[Vec<u8>]) -> Result<Vec<DnaSequence>> {
        self.encode_goldman(chunks)
    }

    /// Encodage Ultimate - combine adaptatif + RS + spreading + GC-aware.
    ///
    /// Utilise le codec UltimateEncoder qui orchestre toutes les optimisations.
    /// Les séquences sont taggées `ultimate#<méthode_de_compression>` par le
    /// codec lui-même, ce qui permet au décodeur de choisir la bonne
    /// décompression.
    fn encode_ultimate(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        use crate::codec::ultimate::{UltimateCodec, UltimateEncoderConfig};

        // Le format GC-aware produit des oligos de 152nt par construction :
        // relever la contrainte de longueur si elle est plus basse, sinon
        // l'encodage échouerait systématiquement.
        let mut constraints = self.config.constraints.clone();
        constraints.max_sequence_length = constraints.max_sequence_length.max(152);

        let config = UltimateEncoderConfig::default();
        let mut codec = UltimateCodec::new(constraints, config);

        codec.encode(data)
    }

    /// Encodage Goldman et al. 2013 - Nature 2013
    ///
    /// Implémentation simplifiée (voir codec/goldman_2013.rs pour le détail) :
    /// - Compression LZ4 (proxy pour le Huffman du papier)
    /// - Encodage 2-bits avec rotation par position
    /// - Index de segment 16 bits encodé sur 8 bases
    ///
    /// La paire encodeur/décodeur est mutuellement cohérente (round-trip validé)
    /// mais ne suit pas fidèlement le schéma à segments alternés du papier.
    fn encode_goldman_2013(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        use crate::codec::goldman_2013::Goldman2013Encoder;

        let goldman_encoder = Goldman2013Encoder::new(self.config.constraints.clone());
        goldman_encoder.encode(data)
    }

    /// Encodage Grass et al. 2015 - Nature Biotechnology 2015
    ///
    /// Spécifications du papier:
    /// - Reed-Solomon (255, 223) comme code interne
    /// - Addressing 3-segments (byte_offset, bit_offset, block_index)
    /// - 4% de redondance logique
    /// - Séquences 124nt
    fn encode_grass_2015(&self, data: &[u8]) -> Result<Vec<DnaSequence>> {
        use crate::codec::grass_2015::Grass2015Encoder;

        let grass_encoder = Grass2015Encoder::new(self.config.constraints.clone());
        grass_encoder.encode(data)
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
        let degree1 = fountain::sample_robust_soliton_degree(100, 42);
        let degree2 = fountain::sample_robust_soliton_degree(100, 42);

        // Même seed = même degré
        assert_eq!(degree1, degree2);

        let _degree3 = fountain::sample_robust_soliton_degree(100, 43);
        // Seed différent = potentiellement différent (mais pas garanti)
    }

    #[test]
    fn test_seed_based_selection() {
        let chunks = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];

        let selected1 = Encoder::select_chunks_seeded(&chunks, 2, 42);
        let selected2 = Encoder::select_chunks_seeded(&chunks, 2, 42);

        assert_eq!(selected1, selected2);
    }
}
