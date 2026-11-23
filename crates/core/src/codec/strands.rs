//! Schéma « brins codés » : adressage dans l'ADN, code Reed-Solomon interne par brin et code
//! externe par bloc de brins.
//!
//! L'architecture à deux niveaux de codes suit celle de Grass et al. (2015) : chaque brin porte
//! un index protégé et une charge utile, et un code externe répare les brins manquants. Les
//! paramètres diffèrent de l'article (corps GF(2^8), 2 bits par nucléotide).
//!
//! Un brin fait 160 nt (40 octets) :
//!
//! | nonce, 3 octets (12 nt) | index, 3 octets (12 nt) | charge utile, 30 octets (120 nt) | parité interne, 4 octets (16 nt) |
//!
//! Les 37 derniers octets forment un mot de code Reed-Solomon (index, charge utile, parité),
//! masqué par un flux pseudo-aléatoire dérivé du nonce avant la conversion en ADN. L'encodeur
//! essaie les nonces jusqu'à obtenir un brin qui respecte les contraintes biochimiques : tous
//! les brins produits y satisfont. Les 24 bits du nonce couvrent plus de 16 millions de
//! candidats par brin, assez pour des contraintes bien plus strictes que celles de l'article.
//!
//! La parité interne corrige jusqu'à 2 octets erronés par lecture. Les brins sont groupés par
//! blocs de 223 ; chaque bloc reçoit 32 brins de parité (code externe, colonne par colonne), ce
//! qui répare jusqu'à 32 brins manquants par bloc. Colonne par colonne, il corrige aussi
//! les octets faux d'un brin présent qu'une seule lecture n'aurait pas su redresser :
//! deux erreurs coûtent la place d'un brin manquant.

use crate::codec::rng::{scramble_u24, SeededRng};
use crate::constraints::DnaConstraints;
use crate::error::{DnaError, Result};
use crate::rs::RsCode;
use crate::sequence::{bytes_to_dna, dna_to_bytes};
use rayon::prelude::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub const NONCE_LEN: usize = 3;
pub const INDEX_LEN: usize = 3;
pub const PAYLOAD_LEN: usize = 30;
pub const INNER_PARITY: usize = 4;
/// Nombre de nonces distincts (le nonce tient sur 3 octets).
pub const MAX_NONCE: u32 = 1 << 24;
/// Longueur du mot de code interne : index, charge utile et parité.
pub const CODEWORD_LEN: usize = INDEX_LEN + PAYLOAD_LEN + INNER_PARITY;
/// Longueur d'un brin, en octets.
pub const STRAND_BYTES: usize = NONCE_LEN + CODEWORD_LEN;
/// Longueur d'un brin, en nucléotides.
pub const STRAND_LEN: usize = STRAND_BYTES * 4;
/// Brins de données par bloc.
pub const OUTER_DATA: usize = 223;
/// Brins de parité par bloc.
pub const OUTER_PARITY: usize = 32;
/// Nombre total de brins que l'index sur 3 octets permet d'adresser.
pub const MAX_STRANDS: u32 = 1 << 24;

const MASK_STREAM: u64 = 0x0000_0000_0000_0012;

/// Répartition d'un corps en brins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrandLayout {
    pub data_strands: u32,
    pub blocks: u32,
    pub total_strands: u32,
}

fn too_large() -> DnaError {
    DnaError::Encoding(format!(
        "fichier trop volumineux pour le schéma brins (limite: {MAX_STRANDS} brins d'index)"
    ))
}

impl StrandLayout {
    /// Répartition pour un corps de `body_len` octets, si le format peut le contenir.
    pub fn for_body(body_len: usize) -> Result<Self> {
        let data_strands =
            u32::try_from(body_len.div_ceil(PAYLOAD_LEN)).map_err(|_| too_large())?;
        let blocks = data_strands.div_ceil(OUTER_DATA as u32);
        let parity = blocks
            .checked_mul(OUTER_PARITY as u32)
            .ok_or_else(too_large)?;
        let total_strands = data_strands
            .checked_add(parity)
            .filter(|&total| total < MAX_STRANDS)
            .ok_or_else(too_large)?;
        Ok(Self {
            data_strands,
            blocks,
            total_strands,
        })
    }

    /// Nombre de brins de données du bloc `block`.
    pub fn data_in_block(&self, block: usize) -> usize {
        let start = block * OUTER_DATA;
        (self.data_strands as usize)
            .saturating_sub(start)
            .min(OUTER_DATA)
    }
}

/// Bilan d'un encodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub data_strands: u32,
    pub parity_strands: u32,
    /// Nonces essayés pour satisfaire les contraintes.
    pub nonces_tried: u64,
}

/// Bilan d'un décodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Lectures de la longueur attendue.
    pub reads_used: usize,
    /// Lectures corrigées par le code interne.
    pub corrected: usize,
    /// Lectures irrécupérables (parité non vérifiée, index hors archive).
    pub uncorrectable: usize,
    /// Brins distincts disponibles avant le code externe.
    pub strands: usize,
    /// Index écartés : lectures en désaccord sur la charge utile d'un même brin.
    pub contested: usize,
    /// Blocs réparés par le code externe.
    pub blocks_repaired: usize,
    /// Brins de données restitués ou corrigés par le code externe.
    pub strands_repaired: usize,
}

/// Flux de masquage, dérivé du nonce tel qu'il est transmis.
fn mask(wire_nonce: u32, out: &mut [u8; CODEWORD_LEN]) {
    SeededRng::new(u64::from(wire_nonce), MASK_STREAM).fill_bytes(out);
}

/// Construit le brin d'index et de charge utile donnés : renvoie son nonce et sa séquence.
fn build_strand(
    index: u32,
    payload: &[u8],
    rs: &RsCode,
    constraints: &DnaConstraints,
    nonces_tried: &mut u64,
    nonce_limit: u32,
) -> Result<(u32, Vec<u8>)> {
    let mut codeword = [0u8; CODEWORD_LEN];
    codeword[..INDEX_LEN].copy_from_slice(&index.to_le_bytes()[..INDEX_LEN]);
    codeword[INDEX_LEN..INDEX_LEN + PAYLOAD_LEN].copy_from_slice(payload);
    let (data, parity) = codeword.split_at_mut(INDEX_LEN + PAYLOAD_LEN);
    rs.parity(data, parity)?;

    let mut stream = [0u8; CODEWORD_LEN];
    let mut bytes = [0u8; STRAND_BYTES];
    let mut dna = Vec::with_capacity(STRAND_LEN);
    for nonce in 0..nonce_limit {
        *nonces_tried += 1;
        // Le nonce est transmis sous une forme dispersée : un nonce brut s'écrirait en
        // longues suites de A, que le criblage rejette.
        let wire_nonce = scramble_u24(nonce);
        bytes[..NONCE_LEN].copy_from_slice(&wire_nonce.to_le_bytes()[..NONCE_LEN]);
        mask(wire_nonce, &mut stream);
        for ((slot, &value), &key) in bytes[NONCE_LEN..]
            .iter_mut()
            .zip(codeword.iter())
            .zip(stream.iter())
        {
            *slot = value ^ key;
        }
        dna.clear();
        bytes_to_dna(&bytes, &mut dna);
        if constraints.satisfied(&dna) {
            return Ok((nonce, dna));
        }
    }
    Err(DnaError::Encoding(format!(
        "aucun nonce ne donne un brin conforme pour l'index {index}: contraintes trop strictes"
    )))
}

/// Décode le mot de code interne d'une lecture pour un nonce transmis donné.
fn parse_with_wire(bytes: &[u8], rs: &RsCode, wire_nonce: u32) -> Result<(u32, Box<[u8]>, usize)> {
    let mut stream = [0u8; CODEWORD_LEN];
    mask(wire_nonce, &mut stream);
    let mut codeword = [0u8; CODEWORD_LEN];
    for ((slot, &value), &key) in codeword
        .iter_mut()
        .zip(bytes[NONCE_LEN..].iter())
        .zip(stream.iter())
    {
        *slot = value ^ key;
    }
    let fixed = rs.correct(&mut codeword)?;
    let index = u32::from(codeword[0]) | u32::from(codeword[1]) << 8 | u32::from(codeword[2]) << 16;
    let payload = Box::from(&codeword[INDEX_LEN..INDEX_LEN + PAYLOAD_LEN]);
    Ok((index, payload, fixed))
}

/// Décode une lecture : renvoie l'index, la charge utile et le nombre d'octets corrigés.
///
/// Une erreur dans le nonce transmis fausse le masque. Si la lecture directe échoue, on essaie
/// chaque variante du nonce à une base près (36 variantes).
fn parse_read(read: &[u8], rs: &RsCode) -> Result<(u32, Box<[u8]>, usize)> {
    let bytes = dna_to_bytes(read)?;
    if bytes.len() != STRAND_BYTES {
        return Err(DnaError::Decoding(
            "lecture de longueur inattendue pour un brin".to_string(),
        ));
    }
    let wire_of =
        |bytes: &[u8]| u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16;
    if let Ok(parsed) = parse_with_wire(&bytes, rs, wire_of(&bytes)) {
        return Ok(parsed);
    }

    let mut candidate = bytes.clone();
    for byte in 0..NONCE_LEN {
        for field in 0..4 {
            let shift = 6 - 2 * field;
            let current = (bytes[byte] >> shift) & 3;
            for replacement in 0u8..4 {
                if replacement == current {
                    continue;
                }
                candidate[byte] = (bytes[byte] & !(3 << shift)) | replacement << shift;
                if let Ok(parsed) = parse_with_wire(&candidate, rs, wire_of(&candidate)) {
                    return Ok(parsed);
                }
            }
        }
    }
    Err(DnaError::Correction(
        "lecture de brin irrécupérable".to_string(),
    ))
}

/// Encode le corps d'une archive en brins (manifestes non compris).
pub fn encode(body: &[u8], constraints: &DnaConstraints) -> Result<(Vec<Vec<u8>>, EncodeStats)> {
    let layout = StrandLayout::for_body(body.len())?;
    let data_strands = layout.data_strands as usize;
    let mut padded = body.to_vec();
    padded.resize(data_strands * PAYLOAD_LEN, 0);

    // Code externe : pour chaque colonne de chaque bloc, la parité des données du bloc.
    let outer = RsCode::new(OUTER_PARITY)?;
    let columns = layout.blocks as usize * PAYLOAD_LEN;
    let parity_bytes: Vec<Vec<u8>> = (0..columns)
        .into_par_iter()
        .map(|column| {
            let block = column / PAYLOAD_LEN;
            let offset = column % PAYLOAD_LEN;
            let count = layout.data_in_block(block);
            let values: Vec<u8> = (0..count)
                .map(|i| padded[(block * OUTER_DATA + i) * PAYLOAD_LEN + offset])
                .collect();
            let encoded = outer.encode(&values).expect("bloc de 223 octets au plus");
            encoded[count..].to_vec()
        })
        .collect();

    let payload_of = |index: usize| -> [u8; PAYLOAD_LEN] {
        if index < data_strands {
            let mut payload = [0u8; PAYLOAD_LEN];
            payload.copy_from_slice(&padded[index * PAYLOAD_LEN..][..PAYLOAD_LEN]);
            payload
        } else {
            let position = index - data_strands;
            let block = position / OUTER_PARITY;
            let parity = position % OUTER_PARITY;
            let mut payload = [0u8; PAYLOAD_LEN];
            for (column, slot) in payload.iter_mut().enumerate() {
                *slot = parity_bytes[block * PAYLOAD_LEN + column][parity];
            }
            payload
        }
    };

    // Essais de nonce au-delà desquels la recherche est vaine : refusée d'emblée si l'espace
    // des nonces ne suffit pas.
    let nonce_limit = constraints
        .search_limit(STRAND_LEN, u64::from(MAX_NONCE))
        .ok_or_else(|| DnaError::Encoding(constraints.too_strict_message(STRAND_LEN)))?;
    let nonce_limit = u32::try_from(nonce_limit)
        .unwrap_or(MAX_NONCE)
        .min(MAX_NONCE);
    let nonces_tried = AtomicU64::new(0);
    let oligos: Vec<Vec<u8>> = (0..layout.total_strands)
        .into_par_iter()
        .map_init(
            || RsCode::new(INNER_PARITY).expect("parité interne valide"),
            |rs, index| {
                let payload = payload_of(index as usize);
                let mut local = 0u64;
                let outcome =
                    build_strand(index, &payload, rs, constraints, &mut local, nonce_limit);
                nonces_tried.fetch_add(local, Ordering::Relaxed);
                outcome.map(|(_, dna)| dna)
            },
        )
        .collect::<Result<Vec<_>>>()?;

    let stats = EncodeStats {
        data_strands: layout.data_strands,
        parity_strands: layout.total_strands - layout.data_strands,
        nonces_tried: nonces_tried.load(Ordering::Relaxed),
    };
    Ok((oligos, stats))
}

/// Contrôle et répare tous les blocs avec le code externe. Rend le nombre de blocs réparés et
/// celui des brins de données restitués ou corrigés.
fn restore_blocks(
    data: &mut [Option<Box<[u8]>>],
    parity: &[Option<Box<[u8]>>],
    outer: &RsCode,
) -> Result<(usize, usize)> {
    // Les blocs sont indépendants : chacun ne touche que ses propres brins.
    let repairs: Vec<Result<usize>> = data
        .par_chunks_mut(OUTER_DATA)
        .zip(parity.par_chunks(OUTER_PARITY))
        .enumerate()
        .map(|(block, (data, parity))| repair_block(block, data, parity, outer))
        .collect();
    let mut totals = (0, 0);
    for repaired in repairs {
        let repaired = repaired?;
        totals.0 += usize::from(repaired > 0);
        totals.1 += repaired;
    }
    Ok(totals)
}

/// Valeur de la colonne `column` dans le brin `offset` du bloc ; 0 pour un brin manquant.
fn byte_of(
    data: &[Option<Box<[u8]>>],
    parity: &[Option<Box<[u8]>>],
    offset: usize,
    column: usize,
) -> u8 {
    let strand = if offset < data.len() {
        &data[offset]
    } else {
        &parity[offset - data.len()]
    };
    strand.as_ref().map_or(0, |payload| payload[column])
}

/// Contrôle ou répare un bloc avec le code externe, colonne par colonne.
///
/// Un brin manquant est un effacement et un octet faux une erreur : chaque colonne admet
/// `2 · erreurs + effacements <= 32`. Rend le nombre de brins de données restitués ou corrigés ;
/// 0 pour un bloc sain.
fn repair_block(
    block: usize,
    data: &mut [Option<Box<[u8]>>],
    parity: &[Option<Box<[u8]>>],
    outer: &RsCode,
) -> Result<usize> {
    let count = data.len();
    let length = count + OUTER_PARITY;
    let missing: Vec<usize> = (0..length)
        .filter(|&offset| !is_present(data, parity, offset))
        .collect();
    let missing_data = missing.iter().filter(|&&offset| offset < count).count();
    if missing_data > OUTER_PARITY {
        return Err(DnaError::InsufficientData(format!(
            "bloc {block} : {missing_data} brins de données manquants pour {OUTER_PARITY} brins de parité"
        )));
    }

    // Écrit dans `codeword` les valeurs de la colonne `column`, dans tous les brins du bloc.
    fn fill(
        codeword: &mut [u8],
        data: &[Option<Box<[u8]>>],
        parity: &[Option<Box<[u8]>>],
        column: usize,
    ) {
        for (offset, slot) in codeword.iter_mut().enumerate() {
            *slot = byte_of(data, parity, offset, column);
        }
    }

    let mut codeword = vec![0u8; length];

    // Un bloc complet dont chaque colonne vérifie la parité est sain.
    if missing.is_empty() {
        let healthy = (0..PAYLOAD_LEN).all(|column| {
            fill(&mut codeword, data, parity, column);
            outer.check(&codeword)
        });
        if healthy {
            return Ok(0);
        }
    }

    let mut touched = vec![false; count];
    for column in 0..PAYLOAD_LEN {
        fill(&mut codeword, data, parity, column);
        outer
            .correct_with_erasures(&mut codeword, &missing)
            .map_err(|e| {
                DnaError::InsufficientData(format!(
                    "bloc {block} : code externe irrécupérable ({e})"
                ))
            })?;
        // Toutes les valeurs corrigées sont relues : un brin présent peut avoir été corrigé.
        for offset in 0..count {
            match &mut data[offset] {
                None => {
                    let mut payload = Box::<[u8]>::from([0u8; PAYLOAD_LEN]);
                    payload[column] = codeword[offset];
                    data[offset] = Some(payload);
                    touched[offset] = true;
                }
                Some(payload) => {
                    if payload[column] != codeword[offset] {
                        payload[column] = codeword[offset];
                        touched[offset] = true;
                    }
                }
            }
        }
    }
    Ok(touched.into_iter().filter(|&touched| touched).count())
}

/// Vrai si le brin `offset` du bloc est présent.
fn is_present(data: &[Option<Box<[u8]>>], parity: &[Option<Box<[u8]>>], offset: usize) -> bool {
    if offset < data.len() {
        data[offset].is_some()
    } else {
        parity[offset - data.len()].is_some()
    }
}

struct IndexEntry {
    payload: Box<[u8]>,
    votes: u32,
    others: u32,
}

/// Décode le corps de l'archive à partir des lectures (manifestes non compris).
pub fn decode(reads: &[Vec<u8>], body_len: usize) -> Result<(Vec<u8>, DecodeStats)> {
    let layout = StrandLayout::for_body(body_len)?;
    let rs = RsCode::new(INNER_PARITY)?;
    let outer = RsCode::new(OUTER_PARITY)?;
    let total = layout.total_strands as usize;
    let mut stats = DecodeStats::default();

    // 1. Code interne : chaque lecture de bonne longueur donne un index et une charge utile.
    let sized: Vec<&Vec<u8>> = reads
        .iter()
        .filter(|read| read.len() == STRAND_LEN)
        .collect();
    stats.reads_used = sized.len();
    let parsed: Vec<(u32, Box<[u8]>, usize)> = sized
        .par_iter()
        .filter_map(|read| match parse_read(read, &rs) {
            Ok((index, payload, fixed)) if index < layout.total_strands => {
                Some((index, payload, fixed))
            }
            _ => None,
        })
        .collect();
    stats.corrected = parsed.iter().filter(|(_, _, fixed)| *fixed > 0).count();
    stats.uncorrectable = sized.len() - parsed.len();

    // 2. Un brin par index ; à égalité ou à la minorité, le brin est écarté.
    let mut strands: HashMap<u32, IndexEntry> = HashMap::with_capacity(parsed.len());
    for (index, payload, _) in parsed {
        match strands.entry(index) {
            Entry::Vacant(slot) => {
                slot.insert(IndexEntry {
                    payload,
                    votes: 1,
                    others: 0,
                });
            }
            Entry::Occupied(mut slot) => {
                let entry = slot.get_mut();
                if entry.payload == payload {
                    entry.votes += 1;
                } else {
                    entry.others += 1;
                }
            }
        }
    }
    let mut slots: Vec<Option<Box<[u8]>>> = vec![None; total];
    for (index, entry) in strands {
        if entry.others >= entry.votes {
            stats.contested += 1;
        } else {
            stats.strands += 1;
            slots[index as usize] = Some(entry.payload);
        }
    }

    // 3. Code externe, bloc par bloc : les brins manquants sont des effacements, les octets faux
    //    des erreurs.
    let data_strands = layout.data_strands as usize;
    let parity = slots.split_off(data_strands);
    let (blocks_repaired, strands_repaired) = restore_blocks(&mut slots, &parity, &outer)?;
    stats.blocks_repaired = blocks_repaired;
    stats.strands_repaired = strands_repaired;

    let mut body = Vec::with_capacity(data_strands * PAYLOAD_LEN);
    for slot in &slots[..data_strands] {
        let payload = slot.as_ref().ok_or_else(|| {
            DnaError::InsufficientData("brin de données manquant après le code externe".to_string())
        })?;
        body.extend_from_slice(payload);
    }
    body.truncate(body_len);
    if body.len() != body_len {
        return Err(DnaError::Decoding(
            "corps reconstitué de longueur incohérente".to_string(),
        ));
    }
    Ok((body, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 5);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    fn shuffled(oligos: &[Vec<u8>], seed: u64) -> Vec<Vec<u8>> {
        let mut out = oligos.to_vec();
        let mut rng = SeededRng::new(seed, 96);
        for i in (1..out.len()).rev() {
            out.swap(i, rng.below(i as u64 + 1) as usize);
        }
        out
    }

    fn flip(oligo: &mut [u8], position: usize) {
        oligo[position] = match oligo[position] {
            b'A' => b'C',
            _ => b'A',
        };
    }

    #[test]
    fn layout_matches_format_limits() {
        let empty = StrandLayout::for_body(0).unwrap();
        assert_eq!(
            empty,
            StrandLayout {
                data_strands: 0,
                blocks: 0,
                total_strands: 0
            }
        );
        let one = StrandLayout::for_body(OUTER_DATA * PAYLOAD_LEN).unwrap();
        assert_eq!(one.data_strands, OUTER_DATA as u32);
        assert_eq!(one.blocks, 1);
        assert_eq!(one.total_strands, 255);
        let two = StrandLayout::for_body(OUTER_DATA * PAYLOAD_LEN + 1).unwrap();
        assert_eq!(two.data_strands, 224);
        assert_eq!(two.blocks, 2);
        assert_eq!(two.total_strands, 288);
        assert_eq!(two.data_in_block(0), OUTER_DATA);
        assert_eq!(two.data_in_block(1), 1);
        assert!(StrandLayout::for_body(usize::MAX).is_err());
        assert!(StrandLayout::for_body(600 * 1024 * 1024).is_err());
        assert!(StrandLayout::for_body(400 * 1024 * 1024).is_ok());
    }

    #[test]
    fn roundtrip_and_every_strand_is_compliant() {
        let constraints = DnaConstraints::default();
        for size in [0usize, 1, 30, 31, 32, 5_000, 100_000] {
            let body = pseudo_random(size, size as u64 + 1);
            let (oligos, stats) = encode(&body, &constraints).unwrap();
            assert_eq!(stats.data_strands, size.div_ceil(PAYLOAD_LEN) as u32);
            assert_eq!(
                stats.parity_strands as usize + stats.data_strands as usize,
                oligos.len()
            );
            for oligo in &oligos {
                assert_eq!(oligo.len(), STRAND_LEN);
                constraints
                    .check(oligo)
                    .unwrap_or_else(|e| panic!("size={size}: {e}"));
            }
            // En moyenne huit essais de nonce par brin : le criblage ne diverge pas.
            if !oligos.is_empty() {
                assert!(stats.nonces_tried < oligos.len() as u64 * 64);
            }
            let (decoded, decode_stats) = decode(&oligos, size).unwrap();
            assert_eq!(decoded, body);
            assert_eq!(decode_stats.blocks_repaired, 0);
            assert_eq!(decode(&shuffled(&oligos, 3), size).unwrap().0, body);
        }
    }

    #[test]
    fn up_to_32_missing_strands_per_block_are_restored() {
        let size = 3 * OUTER_DATA * PAYLOAD_LEN; // trois blocs pleins
        let body = pseudo_random(size, 11);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();
        let data = 3 * OUTER_DATA;

        // 32 brins de données du premier bloc manquants.
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| *index >= 32)
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let (decoded, stats) = decode(&kept, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.blocks_repaired, 1);
        assert_eq!(stats.strands_repaired, 32);

        // 16 données et 16 parités du premier bloc manquantes.
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| !(16..32).contains(index) && !(data..data + 16).contains(index))
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let (decoded, stats) = decode(&kept, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.strands_repaired, 16);

        // Un de plus : le bloc est perdu, l'échec est annoncé.
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| *index >= 33)
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let err = decode(&kept, size).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");

        // Seuls des brins de parité manquent : rien à réparer.
        let kept: Vec<Vec<u8>> = oligos[..data].to_vec();
        let (decoded, stats) = decode(&kept, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.blocks_repaired, 0);
    }

    #[test]
    fn inner_code_corrects_two_bytes_per_read() {
        let size = 2_000;
        let body = pseudo_random(size, 21);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();
        // Deux octets du mot de code erronés : une erreur dans l'index (12 à 24 nt), une dans
        // la charge utile.
        let mut damaged = oligos.clone();
        for oligo in &mut damaged {
            flip(oligo, 16);
            flip(oligo, 100);
        }
        let (decoded, stats) = decode(&damaged, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, oligos.len());
        assert_eq!(stats.uncorrectable, 0);
        assert_eq!(stats.blocks_repaired, 0);
    }

    #[test]
    fn unrecoverable_reads_count_as_missing_strands() {
        let size = 1_000;
        let body = pseudo_random(size, 31);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();
        let mut damaged = oligos.clone();
        for position in [20usize, 60, 100] {
            flip(&mut damaged[4], position);
        }
        let (decoded, stats) = decode(&damaged, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 1);
        assert_eq!(stats.strands_repaired, 1);

        // Nonce altéré d'une base : le masque est faux, mais la variante à une base est
        // retrouvée ; le brin est rendu exact, sans recours au code externe.
        let mut damaged = oligos.clone();
        flip(&mut damaged[7], 2);
        let (decoded, stats) = decode(&damaged, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 0);
        assert_eq!(stats.blocks_repaired, 0);

        // Nonce altéré et trois erreurs dans le mot de code : au-delà de la capacité de
        // correction. La lecture est perdue, puis remplacée par le code externe.
        let mut damaged = oligos.clone();
        flip(&mut damaged[7], 2);
        flip(&mut damaged[7], 40);
        flip(&mut damaged[7], 90);
        flip(&mut damaged[7], 100);
        let (decoded, stats) = decode(&damaged, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 1);
        assert_eq!(stats.strands_repaired, 1);
    }

    #[test]
    fn conflicting_reads_for_one_index_are_settled_by_majority() {
        let size = 1_000;
        let body = pseudo_random(size, 41);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();
        // Lecture forgée : même index, charge utile différente, parité et contraintes valides.
        let rs = RsCode::new(INNER_PARITY).unwrap();
        let (index, _, _) = parse_read(&oligos[5], &rs).unwrap();
        let mut payload = [0u8; PAYLOAD_LEN];
        payload[0] ^= 0xFF;
        let (_, forged) = build_strand(
            index,
            &payload,
            &rs,
            &DnaConstraints::default(),
            &mut 0,
            MAX_NONCE,
        )
        .unwrap();

        // Égalité : le brin est écarté, puis rétabli par le code externe.
        let mut reads = oligos.clone();
        reads.push(forged.clone());
        let (decoded, stats) = decode(&reads, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 1);
        assert_eq!(stats.strands_repaired, 1);

        // Majorité : la bonne charge utile l'emporte.
        let mut reads = oligos.clone();
        reads.push(oligos[5].clone());
        reads.push(forged);
        let (decoded, stats) = decode(&reads, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 0);
        assert_eq!(stats.blocks_repaired, 0);
    }

    /// Une lecture « corrigée » à tort par le code interne : même index, parité interne valide,
    /// autre charge utile. Aucun décodeur ne peut la reconnaître sur une seule lecture.
    fn forged(oligo: &[u8], mask: u8) -> Vec<u8> {
        let rs = RsCode::new(INNER_PARITY).unwrap();
        let (index, mut payload, _) = parse_read(oligo, &rs).unwrap();
        payload[3] ^= mask;
        build_strand(
            index,
            &payload,
            &rs,
            &DnaConstraints::default(),
            &mut 0,
            MAX_NONCE,
        )
        .unwrap()
        .1
    }

    #[test]
    fn a_present_but_false_strand_is_corrected_by_the_outer_code() {
        let size = 3 * OUTER_DATA * PAYLOAD_LEN;
        let body = pseudo_random(size, 61);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();
        let data = 3 * OUTER_DATA;

        // Deux brins faux dans le premier bloc, un dans le deuxième, un brin de parité.
        let mut reads = oligos.clone();
        for (rank, mask) in [(0usize, 0x01u8), (7, 0x80), (300, 0x55), (data + 5, 0xFF)] {
            reads[rank] = forged(&oligos[rank], mask);
        }
        let (decoded, stats) = decode(&reads, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, 0, "leur parité interne est valide");
        assert_eq!(stats.blocks_repaired, 2);
        assert_eq!(
            stats.strands_repaired, 3,
            "les brins de parité ne comptent pas"
        );
    }

    #[test]
    fn a_block_with_too_many_false_strands_is_announced() {
        let size = OUTER_DATA * PAYLOAD_LEN;
        let body = pseudo_random(size, 71);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();

        // Dix-sept brins faux : deux fois plus d'erreurs que la parité n'en corrige.
        let mut reads = oligos.clone();
        for rank in 0..17 {
            reads[rank] = forged(&oligos[rank], 0x01 + rank as u8);
        }
        let err = decode(&reads, size).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");

        // Seize : la limite exacte est atteinte, et le bloc est rétabli.
        let mut reads = oligos.clone();
        for rank in 0..16 {
            reads[rank] = forged(&oligos[rank], 0x01 + rank as u8);
        }
        let (decoded, stats) = decode(&reads, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.blocks_repaired, 1);
        assert_eq!(stats.strands_repaired, 16);
    }

    #[test]
    fn missing_and_false_strands_share_the_capacity_of_the_outer_code() {
        let size = OUTER_DATA * PAYLOAD_LEN;
        let body = pseudo_random(size, 81);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();

        // Un bloc complet : 223 brins de données et 32 de parité.
        let complete = &oligos[..OUTER_DATA + OUTER_PARITY];

        // Vingt-huit manquants et deux faux : 28 + 4 = 32, la limite exacte, et ça passe.
        let mut reads: Vec<Vec<u8>> = complete.to_vec();
        reads[100..128].fill(Vec::new());
        reads[50] = forged(&oligos[50], 0x40);
        reads[51] = forged(&oligos[51], 0x02);
        let (decoded, stats) = decode(&reads, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.strands_repaired, 30);
    }

    #[test]
    fn beyond_the_capacity_of_the_outer_code_no_file_is_ever_returned() {
        use crate::envelope::{Compression, HEADER_LEN};
        use crate::{DecodeOptions, EncodeOptions, Scheme};

        // Un corps d'un bloc exactement, ni compressé ni manifesté dans le corps.
        let size = OUTER_DATA * PAYLOAD_LEN - HEADER_LEN;
        let data = pseudo_random(size, 91);
        let options = EncodeOptions {
            compression: Compression::None,
            ..EncodeOptions::for_scheme(Scheme::Strands)
        };
        let archive = crate::encode(&data, &options).unwrap();
        let first = archive.stats.manifest_copies;

        // Trente brins manquants et deux faux : 30 + 2 × 2 = 34 dépasse les 32 octets de parité.
        // Au-delà de sa capacité, un code Reed-Solomon ne promet pas d'échouer : il peut se
        // tromper en silence. L'enveloppe, elle, ne laisse rien passer.
        let mut reads = archive.oligos.clone();
        reads[first + 100..first + 130].fill(Vec::new());
        reads[first + 50] = forged(&archive.oligos[first + 50], 0x40);
        reads[first + 51] = forged(&archive.oligos[first + 51], 0x02);
        let outcome = crate::decode(&reads, &DecodeOptions::default());
        assert!(
            !outcome.as_ref().is_ok_and(|decoded| decoded.data != data),
            "aucun fichier n'est rendu au-delà de la capacité du code externe"
        );
    }

    #[test]
    fn duplicate_reads_and_wrong_lengths_are_harmless() {
        let size = 500;
        let body = pseudo_random(size, 51);
        let (oligos, _) = encode(&body, &DnaConstraints::default()).unwrap();
        let mut reads: Vec<Vec<u8>> = oligos.iter().flat_map(|o| [o.clone(), o.clone()]).collect();
        reads.push(vec![b'A'; STRAND_LEN - 1]);
        reads.push(vec![b'A'; STRAND_LEN + 4]);
        reads.push(Vec::new());
        let (decoded, stats) = decode(&reads, size).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 0);
        assert_eq!(stats.reads_used, reads.len() - 3);
    }
}
