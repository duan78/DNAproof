//! DNA Fountain (Erlich et Zielinski, *Science* 355:950–954, 2017), en format autosuffisant.
//!
//! Oligo de données, 152 nt avec les paramètres de l'article :
//!
//! | graine, 4 octets (16 nt) | charge utile, 32 octets (128 nt) | parité Reed-Solomon (8 nt) |
//!
//! La charge utile est une *goutte* : le OU exclusif des blocs du corps désignés par la graine.
//! Elle est transmise masquée par un flux dérivé de la graine, puis protégée avec elle par la
//! parité Reed-Solomon. Une lecture qui ne vérifie pas la parité est corrigée si elle porte au
//! plus `parité / 2` octets erronés, et rejetée sinon.
//!
//! Les oligos qui violent les contraintes biochimiques sont écartés (criblage) : on passe à la
//! graine suivante. Avec les contraintes de l'article, environ 12,5 % des candidats passent. Le
//! masquage fait que ce taux tient même pour un fichier minuscule, dont les gouttes seraient
//! sinon presque identiques d'une graine à l'autre.
//!
//! L'encodeur tient un décodeur à jour : l'archive complète est décodable par construction.
//! Avec une redondance `r`, il ajoute des gouttes jusqu'à ce que l'archive reste décodable
//! après l'écart aléatoire de la fraction `1 − 1 / r` des oligos, pour chacun de 256 tirages
//! indépendants (moins pour les très grosses archives). Un code LT a une queue de distribution
//! lourde : sans cette certification, une archive à peine décodable échoue une fois sur dix dès
//! qu'on lui retire des oligos.

use crate::codec::lt::{
    peel, xor_chunks, LayoutSampler, PeelOutcome, PeelingGraph, StructureTracker, DEFAULT_C,
    DEFAULT_DELTA,
};
use crate::codec::manifest::Manifest;
use crate::codec::rng::{scramble_u32, SeededRng};
use crate::constraints::DnaConstraints;
use crate::error::{DnaError, Result};
use crate::rs::RsCode;
use crate::sequence::{bytes_to_dna, dna_to_bytes};
use rayon::prelude::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

/// Taille de la graine, en octets.
pub const SEED_LEN: usize = 4;
/// Taille d'un bloc du corps et de la charge utile d'une goutte, en octets.
pub const CHUNK_LEN: usize = 32;
/// Parité Reed-Solomon par oligo dans l'article : 2 octets, pour rejeter les oligos erronés.
pub const DEFAULT_RS_PARITY: usize = 2;

/// En dessous de ce nombre de graines par lot, l'encodage reste séquentiel.
const PARALLEL_THRESHOLD: usize = 2_048;
const LOSS_STREAM: u64 = 0x0000_0000_0000_0011;

const WHITEN_STREAM: u64 = 0x0000_0000_0000_0013;
/// Marge ajoutée au logarithme du nombre de blocs pour choisir `c` automatiquement.
const COVERAGE_MARGIN: f64 = 6.0;

/// Paramètres du codec DNA Fountain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FountainParams {
    /// Marge de perte. `r` signifie que l'archive reste décodable après la perte aléatoire
    /// d'une fraction allant jusqu'à `1 − 1 / r` des oligos.
    pub redundancy: f64,
    /// Paramètre `c` de la distribution soliton robuste. `None` : choisi automatiquement en
    /// fonction du nombre de blocs (la valeur de l'article dès qu'elle suffit).
    pub c: Option<f64>,
    /// Paramètre `δ` de la distribution soliton robuste.
    pub delta: f64,
    /// Octets de parité Reed-Solomon par oligo.
    pub rs_parity: usize,
}

impl Default for FountainParams {
    fn default() -> Self {
        Self {
            redundancy: 1.10,
            c: None,
            delta: DEFAULT_DELTA,
            rs_parity: DEFAULT_RS_PARITY,
        }
    }
}

impl FountainParams {
    /// Vérifie la cohérence des paramètres.
    pub fn validate(&self) -> Result<()> {
        if !self.redundancy.is_finite() || self.redundancy < 1.0 || self.redundancy > 100.0 {
            return Err(DnaError::InvalidConfig(format!(
                "la redondance doit être un nombre fini entre 1.0 et 100.0 (reçu {})",
                self.redundancy
            )));
        }
        if !self.delta.is_finite() || self.delta <= 0.0 || self.delta >= 1.0 {
            return Err(DnaError::InvalidConfig(format!(
                "δ doit être compris strictement entre 0 et 1 (reçu {})",
                self.delta
            )));
        }
        if self
            .c
            .is_some_and(|c| !c.is_finite() || c <= 0.0 || c > 1.0)
        {
            return Err(DnaError::InvalidConfig(format!(
                "c doit être compris entre 0 et 1 (reçu {})",
                self.c.unwrap_or_default()
            )));
        }
        if self.rs_parity == 0 || self.rs_parity > 32 {
            return Err(DnaError::InvalidConfig(format!(
                "la parité Reed-Solomon doit être comprise entre 1 et 32 octets (reçue {})",
                self.rs_parity
            )));
        }
        Ok(())
    }

    /// Longueur d'un oligo, en nucléotides.
    pub fn oligo_len(&self) -> usize {
        oligo_len(self.rs_parity)
    }

    /// Paramètre `c` retenu pour `chunks` blocs.
    pub fn effective_c(&self, chunks: u32) -> f64 {
        self.c.unwrap_or_else(|| automatic_c(chunks, self.delta))
    }

    /// Fraction des oligos qui doit suffire à décoder.
    fn keep_fraction(&self) -> f64 {
        1.0 / self.redundancy
    }
}

/// Paramètre `c` automatique pour `chunks` blocs.
///
/// La distribution soliton réserve `S = c · ln(K/δ) · √K` gouttes de degré 1 pour `K` blocs. Un
/// bloc n'en reçoit aucune avec une probabilité proche de `e^(−S)` : il faut donc `S` bien plus
/// grand que `ln(K)`. La valeur de l'article (`c = 0,025`) y pourvoit dès quelques milliers de
/// blocs ; en dessous, `c` est relevé pour garder `S ≥ ln(K/δ) + 6`.
pub fn automatic_c(chunks: u32, delta: f64) -> f64 {
    let k = f64::from(chunks.max(1));
    let log = (k / delta).ln();
    let wanted = (log + COVERAGE_MARGIN) / (log * k.sqrt());
    if wanted.is_finite() {
        wanted.clamp(DEFAULT_C, 1.0)
    } else {
        DEFAULT_C
    }
}

/// Longueur d'un oligo pour une parité donnée, en nucléotides.
pub fn oligo_len(rs_parity: usize) -> usize {
    (SEED_LEN + CHUNK_LEN + rs_parity) * 4
}

/// Nombre de blocs nécessaires pour un corps de `body_len` octets. Un corps vide occupe quand
/// même un bloc.
pub fn chunks_for(body_len: usize) -> Option<u32> {
    u32::try_from(body_len.div_ceil(CHUNK_LEN).max(1)).ok()
}

/// Flux de masquage d'une goutte.
fn whiten(seed: u32, out: &mut [u8; CHUNK_LEN]) {
    SeededRng::new(u64::from(seed), WHITEN_STREAM).fill_bytes(out);
}

/// Oligo d'une goutte donnée : graine, charge utile masquée, parité.
#[cfg(test)]
pub(crate) fn oligo_for_droplet(seed: u32, payload: &[u8], rs: &RsCode) -> Result<Vec<u8>> {
    let mut stream = [0u8; CHUNK_LEN];
    whiten(seed, &mut stream);
    let mut block = Vec::with_capacity(SEED_LEN + CHUNK_LEN + rs.ecc());
    block.extend_from_slice(&seed.to_be_bytes());
    block.extend(
        payload
            .iter()
            .zip(stream.iter())
            .map(|(&value, &key)| value ^ key),
    );
    let data_len = block.len();
    block.resize(data_len + rs.ecc(), 0);
    let (head, tail) = block.split_at_mut(data_len);
    rs.parity(head, tail)?;
    let mut dna = Vec::with_capacity(block.len() * 4);
    bytes_to_dna(&block, &mut dna);
    Ok(dna)
}

/// Bilan d'un encodage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EncodeStats {
    pub chunks: u32,
    pub data_oligos: usize,
    pub seeds_tried: u64,
    pub mean_degree: f64,
}

/// Bilan d'un décodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Lectures de la longueur attendue.
    pub reads_used: usize,
    /// Lectures corrigées par la parité Reed-Solomon.
    pub corrected: usize,
    /// Lectures irrécupérables.
    pub uncorrectable: usize,
    /// Gouttes distinctes utilisées.
    pub droplets: usize,
    /// Gouttes écartées : lectures en désaccord sur la charge utile d'une même graine.
    pub contested: usize,
    /// Blocs reconstitués.
    pub chunks_solved: u32,
}

/// Numéro de l'encodage en cours : l'état de travail d'un thread est réutilisé tant que
/// l'encodage est le même, et reconstruit dès que ses paramètres changent.
static ENCODE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

thread_local! {
    static THREAD_STATE: std::cell::RefCell<Option<(u64, Builder)>> =
        const { std::cell::RefCell::new(None) };
}

/// Une goutte produite : l'oligo qui la porte, les blocs qu'il mélange, et sa charge utile.
type Droplet = (Vec<u8>, Vec<u32>, Vec<u8>);

/// État de travail d'un thread d'encodage.
struct Builder {
    sampler: LayoutSampler,
    rs: RsCode,
    indices: Vec<u32>,
    payload: Vec<u8>,
    scratch: Vec<u64>,
    stream: [u8; CHUNK_LEN],
    block: Vec<u8>,
    dna: Vec<u8>,
    constraints: DnaConstraints,
}

impl Builder {
    fn new(k: u32, c: f64, delta: f64, rs: RsCode, constraints: DnaConstraints) -> Result<Self> {
        Ok(Self {
            sampler: LayoutSampler::new(k, c, delta)?,
            rs,
            indices: Vec::new(),
            payload: vec![0u8; CHUNK_LEN],
            scratch: vec![0u64; CHUNK_LEN / 8],
            stream: [0u8; CHUNK_LEN],
            block: Vec::with_capacity(SEED_LEN + CHUNK_LEN + 32),
            dna: Vec::with_capacity((SEED_LEN + CHUNK_LEN + 32) * 4),
            constraints,
        })
    }

    /// Construit la goutte de graine `seed` et retourne son oligo s'il passe le criblage.
    #[allow(clippy::type_complexity)]
    fn build(&mut self, words: &[u64], seed: u32) -> Option<Droplet> {
        self.sampler.layout(seed, &mut self.indices);
        xor_chunks(
            words,
            CHUNK_LEN / 8,
            &self.indices,
            &mut self.scratch,
            &mut self.payload,
        );

        self.block.clear();
        self.block.extend_from_slice(&seed.to_be_bytes());
        self.block.extend_from_slice(&self.payload);
        let data_len = self.block.len();
        self.block.resize(data_len + self.rs.ecc(), 0);
        whiten(seed, &mut self.stream);
        for (slot, &key) in self.block[SEED_LEN..data_len]
            .iter_mut()
            .zip(self.stream.iter())
        {
            *slot ^= key;
        }
        let (head, tail) = self.block.split_at_mut(data_len);
        self.rs.parity(head, tail).ok()?;
        self.dna.clear();
        bytes_to_dna(&self.block, &mut self.dna);
        if !self.constraints.satisfied(&self.dna) {
            return None;
        }
        Some((
            std::mem::take(&mut self.dna),
            self.indices.clone(),
            self.payload.clone(),
        ))
    }
}

/// Nombre de tirages de perte aléatoire qui doivent tous laisser l'archive décodable.
///
/// Chaque tirage revient à un décodage complet : leur nombre baisse avec la taille de l'archive
/// pour que la certification reste une part modeste du temps d'encodage.
fn certification_trials(k: u32) -> u64 {
    if k <= 50_000 {
        256
    } else if k <= 500_000 {
        96
    } else {
        32
    }
}

/// L'archive décode-t-elle une fois la fraction `keep` des gouttes retenue, par tirage `trial` ?
fn decodes_after_loss(graph: &PeelingGraph, keep: f64, trial: u64) -> bool {
    let mut rng = SeededRng::new(trial, LOSS_STREAM);
    graph.solves(|_| rng.next_f64() < keep)
}

/// Joue les tirages de `pending` et n'y garde que ceux qui échouent.
///
/// Ajouter des gouttes ne peut qu'aider : un tirage réussi le reste, il n'est donc plus rejoué.
fn keep_failing_trials(accepted: &[Vec<u32>], k: u32, keep: f64, pending: &mut Vec<u64>) {
    if keep >= 1.0 || pending.is_empty() {
        pending.clear();
        return;
    }
    // Le graphe est construit une seule fois. Chaque tirage, mené sur un thread différent, ne
    // garde que son propre état d'épluchage : reconstruire le graphe par tirage faisait
    // s'effondrer l'encodage des grosses archives faute de mémoire.
    let layouts: Vec<&[u32]> = accepted.iter().map(Vec::as_slice).collect();
    let Ok(graph) = PeelingGraph::new(&layouts, k) else {
        return;
    };
    let failed: Vec<u64> = pending
        .par_iter()
        .filter(|&&trial| !decodes_after_loss(&graph, keep, trial))
        .copied()
        .collect();
    *pending = failed;
}

/// Encode le corps d'une archive en oligos de données (manifestes non compris).
///
/// Le manifeste fait foi pour le nombre de blocs et les paramètres de la distribution : ce sont
/// ceux que le décodeur rejouera.
pub fn encode(
    body: &[u8],
    manifest: &Manifest,
    params: &FountainParams,
    constraints: &DnaConstraints,
) -> Result<(Vec<Vec<u8>>, EncodeStats)> {
    params.validate()?;
    let parity = usize::from(manifest.rs_parity);
    if parity != params.rs_parity || chunks_for(body.len()) != Some(manifest.chunks) {
        return Err(DnaError::InvalidConfig(
            "manifeste incohérent avec les paramètres d'encodage".to_string(),
        ));
    }
    let k = manifest.chunks;

    let mut padded = body.to_vec();
    padded.resize(k as usize * CHUNK_LEN, 0);
    let words: Vec<u64> = padded
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| u64::from_le_bytes(*chunk))
        .collect();
    drop(padded);

    let rs = RsCode::new(parity)?;
    let c = f64::from(manifest.c);
    let delta = f64::from(manifest.delta);
    let mean_degree = LayoutSampler::new(k, c, delta)?.mean_degree();
    let mut probe = Builder::new(k, c, delta, rs.clone(), *constraints)?;

    // La décodabilité ne dépend que de la structure : on suit les blocs résolus, sans valeurs.
    let mut tracker = StructureTracker::new(k)?;
    let mut accepted: Vec<Vec<u32>> = Vec::new();
    let mut oligos: Vec<Vec<u8>> = Vec::new();
    let target = (f64::from(k) * params.redundancy).ceil() as usize;
    let keep = params.keep_fraction();

    let mut sequence: u32 = 0;
    let mut seeds_tried: u64 = 0;
    let mut pass_rate = 1.0 / 8.0;
    // Essais au-delà desquels chercher une goutte conforme est vain : refusés d'emblée s'ils
    // dépassent l'espace des graines. On en admet assez pour deux fois plus de gouttes que
    // visé, plus une marge pour les archives minuscules.
    let oligo_length = oligo_len(parity);
    let per_droplet = constraints
        .search_limit(oligo_length, 1 << 31)
        .ok_or_else(|| DnaError::Encoding(constraints.too_strict_message(oligo_length)))?;
    let max_tries = per_droplet.saturating_mul(2 * target as u64 + 1_000);
    // Tirages de perte qui restent à réussir.
    let mut pending: Vec<u64> = (0..certification_trials(k)).collect();

    loop {
        if seeds_tried >= max_tries {
            return Err(DnaError::Encoding(format!(
                "l'encodage ne converge pas après {seeds_tried} graines essayées pour {k} blocs : \
                 contraintes ou paramètres soliton inadaptés"
            )));
        }
        let complete = tracker.is_complete();
        if complete && oligos.len() >= target {
            keep_failing_trials(&accepted, k, keep, &mut pending);
            if pending.is_empty() {
                break;
            }
        }
        // Tant que l'archive n'est pas décodable, on vise la taille demandée ; ensuite on
        // ajoute par tranches de 2 % jusqu'à ce que tous les tirages de perte réussissent.
        let wanted = if complete {
            (oligos.len() / 50).max(1)
        } else {
            target.saturating_sub(oligos.len()).max(1)
        } as f64;
        let remaining_seeds = u32::MAX - sequence;
        let batch = ((wanted / pass_rate).ceil() as usize)
            .clamp(1_024, 1 << 18)
            .min(remaining_seeds as usize);
        if batch == 0 {
            return Err(DnaError::Encoding("espace de graines épuisé".to_string()));
        }
        let range = sequence..sequence + batch as u32;
        sequence += batch as u32;

        let produced: Vec<Option<Droplet>> = if batch < PARALLEL_THRESHOLD {
            (0..batch)
                .map(|offset| {
                    let seed = scramble_u32(range.start + offset as u32);
                    probe.build(&words, seed)
                })
                .collect()
        } else {
            let code = rs.clone();
            let limits = *constraints;
            let epoch = ENCODE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            (range.start..range.end)
                .into_par_iter()
                .map(|number| {
                    // Un état par thread, gardé tant que l'encodage est le même : le reconstruire
                    // à chaque morceau de travail coûtait plus cher que le travail lui-même.
                    THREAD_STATE.with(|slot| {
                        let mut slot = slot.borrow_mut();
                        if slot.as_ref().is_none_or(|(seen, _)| *seen != epoch) {
                            *slot = Some((
                                epoch,
                                Builder::new(k, c, delta, code.clone(), limits)
                                    .expect("paramètres validés"),
                            ));
                        }
                        let (_, builder) = slot.as_mut().expect("inséré");
                        builder.build(&words, scramble_u32(number))
                    })
                })
                .collect()
        };
        let mut passed = 0usize;
        for (dna, indices, _) in produced.into_iter().flatten() {
            passed += 1;
            tracker.add(&indices)?;
            accepted.push(indices);
            oligos.push(dna);
        }
        seeds_tried += batch as u64;
        pass_rate = ((passed as f64) / batch as f64).clamp(1.0 / 4_096.0, 1.0);
    }

    let data_oligos = oligos.len();
    Ok((
        oligos,
        EncodeStats {
            chunks: k,
            data_oligos,
            seeds_tried,
            mean_degree,
        },
    ))
}

/// Lectures d'une même graine, par statut.
struct SeedEntry {
    /// Lectures reçues sans erreur : (charge utile, nombre de voix).
    clean: Vec<([u8; CHUNK_LEN], u32)>,
    /// Lectures corrigées par la parité : (charge utile, nombre de voix).
    corrected: Vec<([u8; CHUNK_LEN], u32)>,
}

impl SeedEntry {
    fn new(payload: [u8; CHUNK_LEN], corrected: bool) -> Self {
        let entry = Self {
            clean: Vec::new(),
            corrected: Vec::new(),
        };
        let mut entry = entry;
        entry.vote(payload, corrected);
        entry
    }

    /// Compte une lecture, reçue sans erreur ou corrigée par la parité.
    fn vote(&mut self, payload: [u8; CHUNK_LEN], corrected: bool) {
        let list = if corrected {
            &mut self.corrected
        } else {
            &mut self.clean
        };
        match list.iter_mut().find(|(seen, _)| *seen == payload) {
            Some((_, votes)) => *votes += 1,
            None => list.push((payload, 1)),
        }
    }

    /// Ce que la graine retient de ses lectures.
    fn verdict(&self, trusted_only: bool) -> Verdict {
        // Les lectures reçues sans erreur tranchent à la majorité : à égalité, aucune ne
        // l'emporte. Une lecture sans erreur peut être fausse — trois substitutions bien placées
        // forment un autre mot de code, que la parité ne peut pas distinguer —, mais deux
        // lectures fausses de même graine qui s'accordent, cela n'arrive pas : deux voix
        // concordantes suffisent.
        if let Some((payload, votes)) = leader(&self.clean) {
            return if votes >= 2 {
                Verdict::Certain(payload)
            } else {
                Verdict::Likely(payload)
            };
        }
        // Sans aucune lecture propre, on ne croit les lectures corrigées que si elles s'accordent
        // toutes, et jamais en mode `trusted_only`.
        match &self.corrected[..] {
            [single] if !trusted_only => Verdict::Likely(single.0),
            _ => Verdict::None,
        }
    }
}

/// Épluche des gouttes données par leur graine et leur charge utile, dans l'ordre de la liste.
fn peel_droplets(
    droplets: &[(u32, [u8; CHUNK_LEN])],
    k: u32,
    sampler: &mut LayoutSampler,
    indices: &mut Vec<u32>,
) -> Result<PeelOutcome> {
    let mut layouts: Vec<Vec<u32>> = Vec::with_capacity(droplets.len());
    let mut payloads: Vec<u8> = Vec::with_capacity(droplets.len() * CHUNK_LEN);
    for (seed, payload) in droplets {
        sampler.layout(*seed, indices);
        layouts.push(indices.clone());
        payloads.extend_from_slice(payload);
    }
    let references: Vec<&[u32]> = layouts.iter().map(Vec::as_slice).collect();
    peel(&references, &mut payloads, k, CHUNK_LEN)
}

/// Ce qu'une graine retient de ses lectures.
enum Verdict {
    /// Deux lectures reçues sans erreur concordent, au moins : la goutte est certaine.
    Certain([u8; CHUNK_LEN]),
    /// Une seule lecture va dans ce sens : la goutte est probable, et ne complète que.
    Likely([u8; CHUNK_LEN]),
    /// Les lectures ne tranchent pas.
    None,
}

/// La charge utile qui rassemble le plus de voix, et ce nombre, si elle est seule en tête.
fn leader(votes: &[([u8; CHUNK_LEN], u32)]) -> Option<([u8; CHUNK_LEN], u32)> {
    let best = votes.iter().map(|(_, count)| *count).max()?;
    let mut leaders = votes.iter().filter(|(_, count)| *count == best);
    let (payload, votes) = leaders.next()?;
    leaders.next().is_none().then_some((*payload, *votes))
}

/// Décode le corps de l'archive à partir des lectures de données (manifestes exclus).
///
/// Une lecture corrigée par la parité peut l'avoir été à tort : avec deux octets de parité, deux
/// octets erronés se « corrigent » vers un bloc faux une fois sur sept. Dès qu'une graine a des
/// lectures reçues sans erreur, elles seules comptent, à la majorité. Avec `trusted_only`, les
/// graines qui n'en ont aucune sont écartées : si l'intégrité du corps échoue, l'appelant
/// réessaie ainsi.
pub fn decode(
    reads: &[Vec<u8>],
    manifest: &Manifest,
    trusted_only: bool,
) -> Result<(Vec<u8>, DecodeStats)> {
    let parity = usize::from(manifest.rs_parity);
    let rs = RsCode::new(parity)?;
    let expected_len = oligo_len(parity);
    let k = manifest.chunks;
    let body_len = usize::try_from(manifest.body_len)
        .map_err(|_| DnaError::UnsupportedFormat("longueur de corps aberrante".to_string()))?;
    if k == 0 || chunks_for(body_len) != Some(k) {
        return Err(DnaError::UnsupportedFormat(
            "manifeste incohérent : nombre de blocs incompatible avec la longueur du corps"
                .to_string(),
        ));
    }

    // 1. Chaque lecture de la bonne longueur devient un bloc corrigé, ou est rejetée. La graine,
    //    une fois la parité vérifiée, retire le masque de la charge utile.
    let sized: Vec<&Vec<u8>> = reads
        .iter()
        .filter(|read| read.len() == expected_len)
        .collect();
    let decoded: Vec<(u32, [u8; CHUNK_LEN], bool)> = sized
        .par_iter()
        .filter_map(|read| {
            let mut block = dna_to_bytes(read).ok()?;
            let corrected = rs.correct(&mut block).ok()? > 0;
            let seed = u32::from_be_bytes(block[..SEED_LEN].try_into().expect("4 octets"));
            let mut stream = [0u8; CHUNK_LEN];
            whiten(seed, &mut stream);
            let mut payload = [0u8; CHUNK_LEN];
            for index in 0..CHUNK_LEN {
                payload[index] = block[SEED_LEN + index] ^ stream[index];
            }
            Some((seed, payload, corrected))
        })
        .collect();

    let mut stats = DecodeStats {
        reads_used: sized.len(),
        corrected: decoded
            .iter()
            .filter(|(_, _, corrected)| *corrected)
            .count(),
        uncorrectable: sized.len() - decoded.len(),
        ..DecodeStats::default()
    };

    // 2. Une goutte par graine : les lectures reçues sans erreur tranchent à la majorité. Une
    //    lecture « corrigée » par la parité peut l'avoir été à tort (deux octets erronés, pour
    //    deux octets de parité, se corrigent vers un bloc faux une fois sur sept).
    let mut droplets: HashMap<u32, SeedEntry> = HashMap::with_capacity(decoded.len());
    for (seed, payload, corrected) in decoded {
        match droplets.entry(seed) {
            Entry::Vacant(slot) => {
                slot.insert(SeedEntry::new(payload, corrected));
            }
            Entry::Occupied(mut slot) => slot.get_mut().vote(payload, corrected),
        }
    }

    // 3. Épluchage, par graine croissante : le résultat ne dépend ni de l'ordre d'arrivée des
    //    lectures ni d'un tableau de hachage.
    let mut sampler = LayoutSampler::new(k, f64::from(manifest.c), f64::from(manifest.delta))?;
    let mut indices = Vec::new();
    let mut ordered: Vec<(u32, SeedEntry)> = droplets.into_iter().collect();
    ordered.sort_unstable_by_key(|(seed, _)| *seed);
    let mut certain: Vec<(u32, [u8; CHUNK_LEN])> = Vec::new();
    let mut likely: Vec<(u32, [u8; CHUNK_LEN])> = Vec::new();
    for (seed, entry) in ordered {
        match entry.verdict(trusted_only) {
            Verdict::Certain(payload) => certain.push((seed, payload)),
            Verdict::Likely(payload) => likely.push((seed, payload)),
            Verdict::None => stats.contested += 1,
        }
    }

    // Les gouttes certaines seules. Deux lectures fausses de même graine qui s'accordent
    // n'arrive pas : leur solution est exacte, et une goutte probable — une lecture unique,
    // que trois substitutions bien placées peuvent rendre fausse — ne peut pas la corrompre.
    let mut outcome = peel_droplets(&certain, k, &mut sampler, &mut indices)?;
    if outcome.body.is_none() && !likely.is_empty() {
        // Il manque des blocs : les gouttes probables complètent. Leur solution reste à vérifier
        // par l'empreinte, comme toute reconstitution.
        certain.extend(likely);
        certain.sort_unstable_by_key(|(seed, _)| *seed);
        outcome = peel_droplets(&certain, k, &mut sampler, &mut indices)?;
    }
    stats.droplets = certain.len();

    stats.chunks_solved = outcome.solved;
    let Some(mut body) = outcome.body else {
        return Err(DnaError::InsufficientData(format!(
            "{} blocs sur {} reconstitués avec {} gouttes valides : lectures insuffisantes \
             ou trop erronées",
            outcome.solved, k, stats.droplets
        )));
    };
    body.truncate(body_len);
    Ok((body, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Décodage ordinaire : toutes les lectures valides participent.
    fn decode_all(reads: &[Vec<u8>], manifest: &Manifest) -> Result<(Vec<u8>, DecodeStats)> {
        decode(reads, manifest, false)
    }

    use crate::codec::manifest::SchemeId;
    use crate::Compression;
    use std::collections::HashSet;

    fn manifest_for(body: &[u8], params: &FountainParams) -> Manifest {
        Manifest {
            scheme: SchemeId::Fountain,
            rs_parity: params.rs_parity as u8,
            compression: Compression::None,
            body_len: body.len() as u64,
            chunks: chunks_for(body.len()).unwrap(),
            c: params.effective_c(chunks_for(body.len()).unwrap()) as f32,
            delta: params.delta as f32,
        }
    }

    fn encode_body(body: &[u8], params: &FountainParams) -> (Manifest, Vec<Vec<u8>>) {
        let manifest = manifest_for(body, params);
        let (oligos, _) = encode(body, &manifest, params, &DnaConstraints::default()).unwrap();
        (manifest, oligos)
    }

    fn shuffled(oligos: &[Vec<u8>], seed: u64) -> Vec<Vec<u8>> {
        let mut out = oligos.to_vec();
        let mut rng = SeededRng::new(seed, 99);
        for i in (1..out.len()).rev() {
            out.swap(i, rng.below(i as u64 + 1) as usize);
        }
        out
    }

    fn keep_fraction(oligos: &[Vec<u8>], fraction: f64, seed: u64) -> Vec<Vec<u8>> {
        let mut rng = SeededRng::new(seed, 98);
        oligos
            .iter()
            .filter(|_| rng.next_f64() < fraction)
            .cloned()
            .collect()
    }

    fn pseudo_random(len: usize) -> Vec<u8> {
        let mut rng = SeededRng::new(0xA11CE, 7);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    #[test]
    fn roundtrip_and_constraints_for_various_sizes() {
        let params = FountainParams::default();
        for size in [0usize, 1, 31, 32, 33, 1_000, 10_000, 100_000] {
            let body = pseudo_random(size);
            let (manifest, oligos) = encode_body(&body, &params);
            assert!(oligos.len() >= manifest.chunks as usize);
            let constraints = DnaConstraints::default();
            for oligo in &oligos {
                assert_eq!(oligo.len(), 152);
                constraints
                    .check(oligo)
                    .unwrap_or_else(|e| panic!("size={size}: {e}"));
            }
            // Toutes les graines sont distinctes.
            let seeds: HashSet<Vec<u8>> =
                oligos.iter().map(|o| o[..SEED_LEN * 4].to_vec()).collect();
            assert_eq!(seeds.len(), oligos.len());
            let (decoded, stats) = decode_all(&oligos, &manifest).unwrap();
            assert_eq!(decoded, body, "size={size}");
            assert_eq!(stats.chunks_solved, manifest.chunks);
            assert_eq!(stats.corrected, 0);
            assert_eq!(stats.uncorrectable, 0);
            assert_eq!(stats.droplets, oligos.len());
        }
    }

    #[test]
    fn decodes_whatever_the_order() {
        let body = pseudo_random(40_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let mut reversed = oligos.clone();
        reversed.reverse();
        assert_eq!(decode_all(&reversed, &manifest).unwrap().0, body);
        for seed in 0..8u64 {
            assert_eq!(
                decode_all(&shuffled(&oligos, seed), &manifest).unwrap().0,
                body
            );
        }
    }

    #[test]
    fn tolerates_the_loss_implied_by_redundancy() {
        for redundancy in [1.1f64, 1.5, 3.0] {
            let body = pseudo_random(60_000);
            let params = FountainParams {
                redundancy,
                ..FountainParams::default()
            };
            let (manifest, oligos) = encode_body(&body, &params);
            let keep = 1.0 / redundancy;
            // Tirages indépendants de ceux de l'encodeur.
            for seed in 100..108u64 {
                let subset = keep_fraction(&oligos, keep, seed);
                let outcome = decode_all(&subset, &manifest);
                assert!(
                    outcome.is_ok(),
                    "redundancy={redundancy} seed={seed}: {} oligos gardés sur {}",
                    subset.len(),
                    oligos.len()
                );
            }
        }
    }

    #[test]
    fn loss_beyond_the_margin_fails_loudly_never_silently() {
        let body = pseudo_random(60_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        for seed in 0..12u64 {
            // On ne garde que 30 % des oligos : très au-delà de la marge de 9 %.
            let subset = keep_fraction(&oligos, 0.30, seed);
            match decode_all(&subset, &manifest) {
                Ok((decoded, _)) => assert_eq!(decoded, body, "jamais un fichier faux"),
                Err(e) => assert!(matches!(e, DnaError::InsufficientData(_)), "{e}"),
            }
        }
    }

    #[test]
    fn one_error_per_read_is_corrected() {
        let body = pseudo_random(5_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        for position in [3usize, 20, 77, 151] {
            let mut damaged = oligos.clone();
            for oligo in &mut damaged {
                oligo[position] = match oligo[position] {
                    b'A' => b'C',
                    _ => b'A',
                };
            }
            let (decoded, stats) = decode_all(&damaged, &manifest).unwrap();
            assert_eq!(decoded, body);
            assert_eq!(stats.corrected, oligos.len());
            assert_eq!(stats.uncorrectable, 0);
        }
    }

    #[test]
    fn heavily_damaged_reads_are_rejected_and_reported() {
        let body = pseudo_random(5_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let mut damaged = oligos.clone();
        // Trois erreurs dans chaque oligo : irrécupérable avec 2 octets de parité.
        for oligo in &mut damaged {
            for position in [10usize, 60, 110] {
                oligo[position] = match oligo[position] {
                    b'G' => b'T',
                    _ => b'G',
                };
            }
        }
        let outcome = decode_all(&damaged, &manifest);
        match outcome {
            Ok((decoded, stats)) => {
                // Une fausse correction est possible ; elle est alors minoritaire par graine
                // ou sans effet. Le fichier rendu doit être exact.
                assert_eq!(decoded, body);
                assert!(stats.corrected <= oligos.len());
            }
            Err(e) => assert!(matches!(e, DnaError::InsufficientData(_)), "{e}"),
        }
    }

    #[test]
    fn a_clean_read_outweighs_a_falsely_corrected_one() {
        let body = pseudo_random(20_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let rs = RsCode::new(params.rs_parity).unwrap();
        let block = dna_to_bytes(&oligos[0]).unwrap();
        let seed = u32::from_be_bytes(block[..SEED_LEN].try_into().unwrap());
        let mut forged = vec![0u8; SEED_LEN + CHUNK_LEN + rs.ecc()];
        forged[..SEED_LEN].copy_from_slice(&seed.to_be_bytes());
        // Une charge utile différente, mais un oligo valide : deux octets erronés que la
        // parité « corrige » à tort se présentent exactement ainsi après correction.
        forged[SEED_LEN + 5] ^= 0x55;
        let (data, parity) = forged.split_at_mut(SEED_LEN + CHUNK_LEN);
        rs.parity(data, parity).unwrap();
        let mut stream = [0u8; CHUNK_LEN];
        whiten(seed, &mut stream);
        for (slot, &key) in forged[SEED_LEN..SEED_LEN + CHUNK_LEN]
            .iter_mut()
            .zip(stream.iter())
        {
            *slot ^= key;
        }
        let mut falsely_corrected = Vec::new();
        bytes_to_dna(&forged, &mut falsely_corrected);

        // Une lecture sans erreur et une lecture corrigée à tort pour la même graine : la
        // première l'emporte, sans rien perdre.
        let mut reads = oligos.clone();
        reads.push(falsely_corrected);
        let (decoded, stats) = decode_all(&reads, &manifest).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 0);
        assert_eq!(stats.droplets, oligos.len());
    }

    #[test]
    fn two_contradicting_clean_reads_lose_their_droplet() {
        let body = pseudo_random(20_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let block = dna_to_bytes(&oligos[0]).unwrap();
        let seed = u32::from_be_bytes(block[..SEED_LEN].try_into().unwrap());
        let mut payload = [0u8; CHUNK_LEN];
        payload.copy_from_slice(&block[SEED_LEN..SEED_LEN + CHUNK_LEN]);
        payload[3] ^= 0xFF;
        let mut stream = [0u8; CHUNK_LEN];
        whiten(seed, &mut stream);
        for (slot, &key) in payload.iter_mut().zip(stream.iter()) {
            *slot ^= key;
        }
        let forged =
            oligo_for_droplet(seed, &payload, &RsCode::new(params.rs_parity).unwrap()).unwrap();

        // Deux lectures sans erreur, de même graine, qui se contredisent : aucune ne l'emporte.
        // La goutte est écartée, et la redondance la remplace.
        let mut reads = oligos.clone();
        reads.push(forged);
        let (decoded, stats) = decode_all(&reads, &manifest).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 1);
        assert_eq!(stats.droplets, oligos.len() - 1);
    }

    #[test]
    fn a_false_clean_read_is_outvoted_whatever_the_order() {
        let body = pseudo_random(20_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let rs = RsCode::new(params.rs_parity).unwrap();
        let block = dna_to_bytes(&oligos[0]).unwrap();
        let seed = u32::from_be_bytes(block[..SEED_LEN].try_into().unwrap());
        let mut payload = [0u8; CHUNK_LEN];
        payload.copy_from_slice(&block[SEED_LEN..SEED_LEN + CHUNK_LEN]);
        payload[3] ^= 0xFF;
        let mut stream = [0u8; CHUNK_LEN];
        whiten(seed, &mut stream);
        for (slot, &key) in payload.iter_mut().zip(stream.iter()) {
            *slot ^= key;
        }
        // Un oligo valide, d'une autre charge utile : c'est ainsi que se présente une lecture à
        // trois substitutions qui forme un autre mot de code. La parité la croit intacte.
        let false_clean = oligo_for_droplet(seed, &payload, &rs).unwrap();
        assert!(rs.check(&dna_to_bytes(&false_clean).unwrap()));

        // Deux lectures exactes contre une fausse : la goutte est retenue, juste, dans les deux
        // ordres d'arrivée.
        for order in [0, 1] {
            let mut reads = oligos.clone();
            reads.push(oligos[0].clone());
            if order == 0 {
                reads.push(false_clean.clone());
            } else {
                reads.insert(0, false_clean.clone());
            }
            let (decoded, stats) = decode_all(&reads, &manifest).unwrap();
            assert_eq!(decoded, body, "ordre {order}");
            assert_eq!(stats.contested, 0, "ordre {order}");
            assert_eq!(stats.droplets, oligos.len(), "ordre {order}");
        }
    }

    #[test]
    fn a_lone_false_read_cannot_corrupt_a_well_covered_archive() {
        // Régression : une lecture à trois substitutions peut former un autre mot de code, avec
        // une graine qu'aucun oligo de l'archive ne porte. La parité la croit intacte, elle est
        // seule de son avis, et une seule équation fausse corrompait des milliers de blocs.
        let body = pseudo_random(20_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let rs = RsCode::new(params.rs_parity).unwrap();
        // Un oligo de règle, porteur d'une graine étrangère à l'archive.
        let false_read = oligo_for_droplet(0xEB1A_08F7, &[0x5A; CHUNK_LEN], &rs).unwrap();
        assert!(rs.check(&dna_to_bytes(&false_read).unwrap()));

        // Deux lectures exactes par goutte : chacune est certaine, et l'archive se résout sans
        // la lecture fausse, en mode ordinaire comme en mode `trusted_only`.
        for trusted_only in [false, true] {
            let mut reads: Vec<Vec<u8>> =
                oligos.iter().flat_map(|o| [o.clone(), o.clone()]).collect();
            reads.push(false_read.clone());
            let (decoded, stats) = decode(&reads, &manifest, trusted_only).unwrap();
            assert_eq!(decoded, body, "trusted_only = {trusted_only}");
            assert_eq!(stats.contested, 0);
            // La lecture fausse a bien été vue comme une goutte probable, et écartée du calcul.
            assert_eq!(
                stats.droplets,
                oligos.len(),
                "trusted_only = {trusted_only}"
            );
        }
    }

    #[test]
    fn the_trusted_pass_never_follows_a_corrected_read() {
        let body = pseudo_random(20_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        // Chaque lecture reçoit exactement une erreur : la parité la corrige, à raison.
        let mut damaged = oligos.clone();
        for oligo in &mut damaged {
            oligo[40] = match oligo[40] {
                b'A' => b'C',
                _ => b'A',
            };
        }
        let (decoded, stats) = decode_all(&damaged, &manifest).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, oligos.len());

        // En mode `trusted_only`, il n'y a aucune lecture propre : le second passage n'utilise
        // jamais une lecture corrigée, même juste. L'échec est annoncé.
        let error = decode(&damaged, &manifest, true).unwrap_err();
        assert!(matches!(error, DnaError::InsufficientData(_)), "{error}");
    }

    #[test]
    fn duplicate_reads_are_harmless() {
        let body = pseudo_random(3_000);
        let params = FountainParams::default();
        let (manifest, oligos) = encode_body(&body, &params);
        let reads: Vec<Vec<u8>> = oligos
            .iter()
            .flat_map(|o| [o.clone(), o.clone(), o.clone()])
            .collect();
        assert_eq!(decode_all(&reads, &manifest).unwrap().0, body);
    }

    #[test]
    fn inconsistent_manifest_is_rejected() {
        let body = pseudo_random(3_000);
        let params = FountainParams::default();
        let (mut manifest, oligos) = encode_body(&body, &params);
        manifest.chunks += 1;
        assert!(matches!(
            decode_all(&oligos, &manifest),
            Err(DnaError::UnsupportedFormat(_))
        ));
        let (mut manifest, oligos) = encode_body(&body, &params);
        manifest.chunks = 0;
        assert!(matches!(
            decode_all(&oligos, &manifest),
            Err(DnaError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn more_parity_corrects_more_errors_per_read() {
        let body = pseudo_random(8_000);
        let params = FountainParams {
            rs_parity: 8,
            ..FountainParams::default()
        };
        let (manifest, mut oligos) = encode_body(&body, &params);
        assert_eq!(params.oligo_len(), 176);
        for oligo in &mut oligos {
            for position in [5usize, 50, 95, 140] {
                oligo[position] = match oligo[position] {
                    b'T' => b'A',
                    _ => b'T',
                };
            }
        }
        let (decoded, stats) = decode_all(&oligos, &manifest).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, oligos.len());
    }

    #[test]
    fn params_are_validated() {
        let constraints = DnaConstraints::default();
        let mut params = FountainParams {
            redundancy: 0.5,
            ..FountainParams::default()
        };
        assert!(params.validate().is_err());
        params.redundancy = f64::NAN;
        assert!(params.validate().is_err());
        params.redundancy = 1.5;
        params.rs_parity = 0;
        assert!(params.validate().is_err());
        params.rs_parity = 40;
        assert!(params.validate().is_err());
        let body = b"x".to_vec();
        params = FountainParams::default();
        let mut manifest = manifest_for(&body, &params);
        manifest.rs_parity = 4;
        assert!(encode(&body, &manifest, &params, &constraints).is_err());
        assert!(chunks_for(0) == Some(1));
        assert!(chunks_for(33) == Some(2));
    }
}
