//! Schéma « goldman » : code de Huffman ternaire, ADN sans base répétée, fenêtres recouvrantes
//! et index à trit de parité (Goldman et al., 2013).
//!
//! La méthode est celle de l'information complémentaire de l'article, étape par étape :
//!
//! 1. Les octets deviennent des trits (chiffres de 0 à 2) par un code de Huffman ternaire.
//! 2. Le filet de trits se termine par du bourrage, puis par sa longueur sur 20 trits, de sorte
//!    que sa longueur soit un multiple de 25.
//! 3. Chaque trit choisit l'une des trois bases différentes de la précédente : deux bases
//!    identiques ne se suivent jamais.
//! 4. L'ADN est relu par fenêtres de 100 nt décalées de 25 : chaque base apparaît dans quatre
//!    fenêtres.
//! 5. Une fenêtre sur deux est prise en sens inverse ; chaque brin reçoit un index de 15 trits
//!    (identifiant de fichier, adresse de la fenêtre, trit de parité) et deux bases de repère,
//!    qui indiquent dans quel sens il a été lu.
//!
//! | repère A ou T | message, 100 nt | index, 15 nt | repère C ou G |  (117 nt)
//!
//! Deux adaptations au format v2. Le code de Huffman est construit à partir des octets du
//! fichier et porté par l'archive (l'article utilisait un code fixe, transmis à part) ; quand il
//! ne rapporte pas, un code fixe de 5 ou 6 trits prend le relais. Et 75 nt de bourrage entourent
//! le message, pour que ses extrémités bénéficient elles aussi du recouvrement quadruple.
//!
//! Contrairement aux autres schémas, celui-ci ne règle pas la teneur en GC de ses brins : elle
//! est de 50 % en moyenne, sans garantie par brin, comme dans l'article. Sa robustesse vient du
//! recouvrement : chaque base du message est lue quatre fois, dans quatre brins différents, et
//! c'est le vote qui tranche. Une position sans lecture ou mise à égalité arrête le décodage.

use crate::codec::rng::scramble_u32;
use crate::error::{DnaError, Result};
use crate::sequence::{base_to_bits, complement, reverse_complement, BASES};
use rayon::prelude::*;

/// Longueur d'un brin, en nucléotides.
pub const STRAND_LEN: usize = 117;
/// Longueur du message porté par un brin : une fenêtre de l'ADN encodé.
pub const SEGMENT_LEN: usize = 100;
/// Décalage entre deux fenêtres consécutives.
pub const SEGMENT_STEP: usize = 25;
/// Nombre maximal de fenêtres : l'adresse tient sur 12 trits.
pub const MAX_SEGMENTS: usize = 531_441;

const ID_TRITS: usize = 2;
const ADDRESS_TRITS: usize = 12;
const INDEX_TRITS: usize = ID_TRITS + ADDRESS_TRITS + 1;
/// Brin sans ses deux bases de repère.
const CORE_LEN: usize = SEGMENT_LEN + INDEX_TRITS;
/// Longueur du filet de trits, inscrite à sa fin.
const LENGTH_TRITS: usize = 20;
/// Fenêtres de bourrage de part et d'autre du message : chaque position du message est alors
/// couverte quatre fois, la première et la dernière comprises.
const EDGE_WINDOWS: usize = 3;
const EDGE_PAD: usize = EDGE_WINDOWS * SEGMENT_STEP;
/// Longueur maximale d'un code de Huffman, en trits (3^80 tient dans 128 bits).
const MAX_CODE_LEN: usize = 80;
/// Modes de codage inscrits en tête du message : code fixe, ou code de Huffman de l'archive.
const MODE_FIXED: u8 = 0;
const MODE_HUFFMAN: u8 = 1;

/// Bourrage de gauche : 75 nt sans répétition, terminés par A, la ligne dont part le premier
/// trit du message.
const LEFT_PAD: [u8; EDGE_PAD] = {
    let mut pad = [0u8; EDGE_PAD];
    let mut position = 0;
    while position < EDGE_PAD {
        pad[position] = b"CGA"[position % 3];
        position += 1;
    }
    pad
};

/// Paramètres du schéma.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GoldmanParams {
    /// Identifiant du fichier, de 0 à 8 : il sépare plusieurs archives mélangées dans un même
    /// tube, et le décodage n'en retient qu'une.
    pub file_id: u8,
}

impl GoldmanParams {
    /// Vérifie la cohérence des paramètres.
    pub fn validate(&self) -> Result<()> {
        if self.file_id > 8 {
            return Err(DnaError::InvalidConfig(format!(
                "identifiant de fichier invalide: {} (attendu: 0 à 8)",
                self.file_id
            )));
        }
        Ok(())
    }
}

/// Répartition d'un message en fenêtres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentLayout {
    /// Longueur de l'ADN encodé, en nucléotides.
    pub length: usize,
    pub segments: u32,
}

fn too_large() -> DnaError {
    DnaError::Encoding(format!(
        "fichier trop volumineux pour le schéma goldman (limite: {MAX_SEGMENTS} fenêtres, \
         environ 2 Mo)"
    ))
}

fn inconsistent(what: &str) -> DnaError {
    DnaError::Integrity(format!("message ternaire incohérent : {what}"))
}

impl SegmentLayout {
    /// Répartition d'un filet de trits donné sans sa longueur ni son bourrage.
    pub fn for_message(len: usize) -> Result<Self> {
        let Some(with_length) = len.checked_add(LENGTH_TRITS) else {
            return Err(too_large());
        };
        let Some(length) = with_length
            .max(SEGMENT_LEN)
            .div_ceil(SEGMENT_STEP)
            .checked_mul(SEGMENT_STEP)
        else {
            return Err(too_large());
        };
        let segments = length / SEGMENT_STEP + EDGE_WINDOWS;
        if segments > MAX_SEGMENTS {
            return Err(too_large());
        }
        Ok(Self {
            length,
            segments: segments as u32,
        })
    }

    /// Répartition annoncée par le manifeste d'une archive de `body_len` octets.
    pub fn from_manifest(segments: u32, body_len: u64) -> Result<Self> {
        // Au moins une fenêtre de message : 100 nt, et les trois du bourrage de gauche.
        let smallest = EDGE_WINDOWS + SEGMENT_LEN / SEGMENT_STEP;
        let Some(windows) = usize::try_from(segments)
            .ok()
            .filter(|&segments| (smallest..=MAX_SEGMENTS).contains(&segments))
        else {
            return Err(DnaError::UnsupportedFormat(format!(
                "nombre de fenêtres aberrant: {segments}"
            )));
        };
        // Un octet vaut au moins un trit : le corps ne peut pas être plus long que l'ADN.
        if body_len > (windows - EDGE_WINDOWS) as u64 * SEGMENT_STEP as u64 {
            return Err(DnaError::UnsupportedFormat(
                "longueur de corps aberrante pour ce nombre de fenêtres".to_string(),
            ));
        }
        Ok(Self {
            length: (windows - EDGE_WINDOWS) * SEGMENT_STEP,
            segments,
        })
    }
}

/// Longueur en trits de chacun des 256 octets d'un code préfixe ternaire.
type Lengths = [u8; 256];

/// Code préfixe ternaire canonique : un octet devient une suite de 1 à 80 trits.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TernaryCode {
    lengths: Box<Lengths>,
    /// Code de chaque octet, dans ses `lengths` trits de poids faible.
    codes: Box<[u128; 256]>,
    /// Nombre de symboles de chaque longueur.
    counts: [u16; MAX_CODE_LEN + 1],
    /// Premier code de chaque longueur.
    first_code: [u128; MAX_CODE_LEN + 1],
    /// Rang, parmi les symboles rangés par longueur puis par valeur, du premier de chaque
    /// longueur.
    first_index: [u16; MAX_CODE_LEN + 1],
    /// Symboles rangés par longueur, puis par valeur.
    symbols: Box<[u8]>,
    max_len: usize,
}

fn pow3(exponent: usize) -> u128 {
    3u128.pow(exponent as u32)
}

impl TernaryCode {
    /// Code fixe : 5 trits pour les octets 0 à 235, 6 pour les 20 autres. Un mot de 6 trits
    /// reste inutilisé, qui signale une lecture fausse.
    fn fixed() -> Self {
        let mut lengths = [0u8; 256];
        for (symbol, slot) in lengths.iter_mut().enumerate() {
            *slot = if symbol < 236 { 5 } else { 6 };
        }
        Self::from_lengths(&lengths).expect("code fixe valide")
    }

    /// Code canonique de longueurs données (0 : symbole absent).
    fn from_lengths(lengths: &Lengths) -> Result<Self> {
        let max_len = *lengths.iter().max().unwrap_or(&0) as usize;
        if max_len > MAX_CODE_LEN {
            return Err(inconsistent("code de plus de 80 trits"));
        }
        let mut counts = [0u16; MAX_CODE_LEN + 1];
        for &len in lengths.iter() {
            // Les symboles absents (longueur 0) ne prennent aucune place.
            if len > 0 {
                counts[usize::from(len)] += 1;
            }
        }
        // Inégalité de Kraft, en entiers : la somme des comptes, pondérée par 3^(longueur
        // maximale moins la leur), ne dépasse pas 3^(longueur maximale).
        let weighted: u128 = counts[1..=max_len]
            .iter()
            .zip((0..max_len).rev())
            .map(|(&count, exponent)| u128::from(count) * pow3(exponent))
            .sum();
        if weighted > pow3(max_len) {
            return Err(inconsistent("longueurs de code sans code préfixe"));
        }
        let mut symbols: Vec<u8> = (0..=255u8)
            .filter(|&s| lengths[usize::from(s)] > 0)
            .collect();
        symbols.sort_unstable_by_key(|&symbol| (lengths[usize::from(symbol)], symbol));

        let mut code = 0u128;
        let mut index = 0usize;
        let mut first_code = [0u128; MAX_CODE_LEN + 1];
        let mut first_index = [0u16; MAX_CODE_LEN + 1];
        let mut codes = Box::new([0u128; 256]);
        for len in 1..=max_len {
            code = (code + u128::from(counts[len - 1])) * 3;
            first_code[len] = code;
            first_index[len] = u16::try_from(index).map_err(|_| inconsistent("code trop grand"))?;
            let group = &symbols[index..index + usize::from(counts[len])];
            for (rank, &symbol) in group.iter().enumerate() {
                codes[usize::from(symbol)] = code + rank as u128;
            }
            index += group.len();
        }
        Ok(Self {
            lengths: Box::new(*lengths),
            codes,
            counts,
            first_code,
            first_index,
            symbols: symbols.into_boxed_slice(),
            max_len,
        })
    }

    /// Nombre de trits que le code donne à `bytes`.
    fn len_of(&self, bytes: &[u8]) -> usize {
        bytes
            .iter()
            .map(|&byte| usize::from(self.lengths[usize::from(byte)]))
            .sum()
    }

    /// Écrit les trits de `bytes`, poids fort en tête.
    fn encode(&self, bytes: &[u8], out: &mut Vec<u8>) {
        let mut trits = [0u8; MAX_CODE_LEN];
        for &byte in bytes {
            let len = usize::from(self.lengths[usize::from(byte)]);
            let mut code = self.codes[usize::from(byte)];
            for slot in trits[..len].iter_mut().rev() {
                *slot = (code % 3) as u8;
                code /= 3;
            }
            out.extend_from_slice(&trits[..len]);
        }
    }

    /// Lit `count` symboles au début de `trits`. Rend les symboles et les trits restants.
    fn take<'a>(&self, trits: &'a [u8], count: usize) -> Result<(Vec<u8>, &'a [u8])> {
        let mut out = Vec::with_capacity(count.min(trits.len()));
        let mut position = 0;
        while out.len() < count {
            // Flux épuisé : normal quand on lit jusqu'au bout, une troncature sinon.
            if position == trits.len() {
                if count == usize::MAX {
                    break;
                }
                return Err(inconsistent("filet de trits tronqué"));
            }
            let mut value = 0u128;
            let mut len = 0;
            let mut symbol = None;
            while position < trits.len() {
                let trit = trits[position];
                if trit > 2 {
                    return Err(inconsistent("chiffre hors de 0, 1, 2"));
                }
                value = value * 3 + u128::from(trit);
                position += 1;
                len += 1;
                if len > self.max_len {
                    return Err(inconsistent("code trop long"));
                }
                let first = self.first_code[len];
                let size = u128::from(self.counts[len]);
                if size > 0 && value >= first && value < first + size {
                    let rank = usize::try_from(value - first).expect("dans l'intervalle");
                    symbol = Some(self.symbols[usize::from(self.first_index[len]) + rank]);
                    break;
                }
            }
            out.push(symbol.ok_or_else(|| inconsistent("code inconnu"))?);
        }
        Ok((out, &trits[position..]))
    }
}

/// Longueurs de code d'un code de Huffman ternaire pour des comptes donnés.
///
/// `None` si un code dépasserait [`MAX_CODE_LEN`] trits (il faudrait pour cela des comptes
/// dignes de Fibonacci sur plus de 10^16 octets).
fn huffman_lengths(counts: &[u64; 256]) -> Option<Lengths> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let leaves: Vec<usize> = counts
        .iter()
        .enumerate()
        .filter(|(_, &count)| count > 0)
        .map(|(symbol, _)| symbol)
        .collect();
    let mut lengths = [0u8; 256];
    match leaves.as_slice() {
        [] => return Some(lengths),
        &[only] => {
            lengths[only] = 1;
            return Some(lengths);
        }
        _ => {}
    }

    // Un arbre ternaire complet a un nombre de feuilles impair : on ajoute au besoin une
    // feuille de poids nul, qui ne reçoit jamais de symbole.
    let dummies = (leaves.len() + 1) % 2;
    let mut parents: Vec<Option<usize>> = vec![None; leaves.len() + dummies];
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
    for (id, &symbol) in leaves.iter().enumerate() {
        heap.push(Reverse((counts[symbol], id)));
    }
    for id in leaves.len()..leaves.len() + dummies {
        heap.push(Reverse((0, id)));
    }
    let mut next = parents.len();
    while heap.len() > 1 {
        let mut merged = 0u64;
        for _ in 0..3 {
            let Reverse((count, id)) = heap.pop().expect("au moins trois nœuds");
            parents[id] = Some(next);
            merged += count;
        }
        parents.push(None);
        heap.push(Reverse((merged, next)));
        next += 1;
    }
    for (id, &symbol) in leaves.iter().enumerate() {
        let mut depth = 0usize;
        let mut node = id;
        while let Some(parent) = parents[node] {
            depth += 1;
            node = parent;
        }
        if depth > MAX_CODE_LEN {
            return None;
        }
        lengths[symbol] = u8::try_from(depth).ok()?;
    }
    Some(lengths)
}

/// Nombre de trits du mode et de la table des longueurs : 256 octets de 0 à 80 en prennent 5.
const TABLE_TRITS: usize = 5 * 257;

/// Encode le corps en filet de trits, précédé de son mode de codage. Rend vrai si le code de
/// Huffman de l'archive a été retenu.
///
/// Le mode et la table s'écrivent avec le code fixe : le décodeur les lit avant de connaître le
/// code du reste.
fn encode_message(body: &[u8]) -> Result<(bool, Vec<u8>)> {
    let fixed = TernaryCode::fixed();
    let mut counts = [0u64; 256];
    for &byte in body {
        counts[usize::from(byte)] += 1;
    }
    // Le code de Huffman ne sert que s'il économise plus de trits que sa table n'en coûte.
    let huffman = huffman_lengths(&counts)
        .and_then(|lengths| TernaryCode::from_lengths(&lengths).ok())
        .filter(|code| code.len_of(body) + TABLE_TRITS < fixed.len_of(body));
    let Some(code) = huffman else {
        let mut plain = Vec::new();
        fixed.encode(&[MODE_FIXED], &mut plain);
        fixed.encode(body, &mut plain);
        return Ok((false, plain));
    };
    let mut trits = Vec::with_capacity(code.len_of(body) + TABLE_TRITS);
    fixed.encode(&[MODE_HUFFMAN], &mut trits);
    fixed.encode(code.lengths.as_slice(), &mut trits);
    code.encode(body, &mut trits);
    Ok((true, trits))
}

/// Décode le filet de trits d'un message, qui doit être épuisé exactement.
fn decode_message(trits: &[u8]) -> Result<Vec<u8>> {
    let fixed = TernaryCode::fixed();
    let (mode, rest) = fixed.take(trits, 1)?;
    let (code, rest) = match mode[0] {
        MODE_FIXED => (fixed.clone(), rest),
        MODE_HUFFMAN => {
            let (table, rest) = fixed.take(rest, 256)?;
            let lengths: Lengths = table.as_slice().try_into().expect("256 octets");
            (TernaryCode::from_lengths(&lengths)?, rest)
        }
        _ => return Err(inconsistent("mode de codage inconnu")),
    };
    let (body, rest) = code.take(rest, usize::MAX)?;
    if !rest.is_empty() {
        return Err(inconsistent("trits sans symbole"));
    }
    Ok(body)
}

/// Écrit l'ADN d'un filet de trits : chaque trit choisit une base différente de la précédente,
/// la première ligne étant celle de `previous`.
fn trits_to_dna(trits: &[u8], previous: u8, out: &mut Vec<u8>) {
    let mut index = usize::from(base_to_bits(previous).expect("base standard"));
    for &trit in trits.iter() {
        index = (index + 1 + usize::from(trit)) % 4;
        out.push(BASES[index]);
    }
}

/// Rang d'une base dans `BASES`, et trit qui mène de `previous` (son rang) à `base`.
fn base_rank(base: u8) -> Result<usize> {
    Ok(usize::from(
        base_to_bits(base).ok_or_else(|| inconsistent("base inconnue"))?,
    ))
}

fn trit_between(previous: usize, base: u8) -> Result<usize> {
    let value = (base_rank(base)? + 4 - previous - 1) % 4;
    if value == 3 {
        // Deux bases identiques de suite : impossible dans un brin conçu.
        return Err(inconsistent("bases répétées"));
    }
    Ok(value)
}

/// Index d'un brin : identifiant de fichier, adresse de la fenêtre, trit de parité.
fn index_trits(segment: usize, file_id: u8) -> [u8; INDEX_TRITS] {
    let mut address = [0u8; ADDRESS_TRITS];
    let mut value = segment;
    for slot in address.iter_mut().rev() {
        *slot = (value % 3) as u8;
        value /= 3;
    }
    // La parité est la somme, modulo 3, du premier trit de l'identifiant et des trits
    // d'adresse en position impaire (le premier trit est en position 1).
    let mut parity = file_id / 3;
    for &trit in address.iter().step_by(2) {
        parity = (parity + trit) % 3;
    }
    let mut index = [0u8; INDEX_TRITS];
    index[..ID_TRITS].copy_from_slice(&[file_id / 3, file_id % 3]);
    index[ID_TRITS..ID_TRITS + ADDRESS_TRITS].copy_from_slice(&address);
    index[INDEX_TRITS - 1] = parity;
    index
}

/// Construit le brin d'une fenêtre (étapes 1.6 à 1.9).
fn strand(segment: usize, window: &[u8], file_id: u8) -> Vec<u8> {
    // 1.6 : une fenêtre impaire est prise en sens inverse.
    let message: Vec<u8> = if segment % 2 == 1 {
        reverse_complement(window)
    } else {
        window.to_vec()
    };
    let mut core = Vec::with_capacity(CORE_LEN);
    core.extend_from_slice(&message);
    // 1.8 : l'index suit le message, à partir de sa dernière base.
    trits_to_dna(
        &index_trits(segment, file_id),
        message[SEGMENT_LEN - 1],
        &mut core,
    );
    // 1.9 : deux bases de repère, sans répétition. Le choix libre dépend du numéro de fenêtre :
    // un même fichier donne toujours les mêmes brins.
    let bits = scramble_u32(segment as u32);
    let start = match core[0] {
        b'A' => b'T',
        b'T' => b'A',
        _ if bits & 1 == 0 => b'A',
        _ => b'T',
    };
    let end = match core[CORE_LEN - 1] {
        b'C' => b'G',
        b'G' => b'C',
        _ if bits & 0b10 == 0 => b'C',
        _ => b'G',
    };
    let mut out = Vec::with_capacity(STRAND_LEN);
    out.push(start);
    out.extend_from_slice(&core);
    out.push(end);
    out
}

/// Bilan d'un encodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeStats {
    /// Fenêtres, c'est-à-dire brins de données.
    pub segments: u32,
    /// Vrai si le corps s'encode avec un code de Huffman construit pour l'archive.
    pub huffman: bool,
}

/// Bilan d'un décodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Lectures de la longueur d'un brin.
    pub reads_used: usize,
    /// Lectures conservées malgré une erreur repérée par une répétition de bases.
    pub corrected: usize,
    /// Lectures écartées : index illisible, ou d'une autre archive.
    pub uncorrectable: usize,
    /// Fenêtres distinctes qui ont contribué au vote.
    pub segments: usize,
    /// Positions mises à égalité.
    pub contested: usize,
}

/// Encode le corps d'une archive en brins (manifestes non compris).
pub fn encode(body: &[u8], params: &GoldmanParams) -> Result<(Vec<Vec<u8>>, EncodeStats)> {
    let (huffman, trits) = encode_message(body)?;
    let layout = SegmentLayout::for_message(trits.len())?;

    // 1.3 : S₁, puis du bourrage nul, puis la longueur de S₁ sur 20 trits.
    let mut trits = trits;
    let len_s1 = trits.len();
    trits.resize(layout.length - LENGTH_TRITS, 0);
    let mut value = len_s1;
    let mut length = [0u8; LENGTH_TRITS];
    for slot in length.iter_mut().rev() {
        *slot = (value % 3) as u8;
        value /= 3;
    }
    trits.extend_from_slice(&length);

    // 1.4 : l'ADN, sans base répétée, à partir de la ligne « A ».
    let mut dna = Vec::with_capacity(layout.length);
    trits_to_dna(&trits, b'A', &mut dna);

    // Bourrage : à gauche une suite fixe terminée par A ; à droite, la suite de trits nuls qui
    // suit la dernière base du message.
    let mut padded = Vec::with_capacity(layout.length + 2 * EDGE_PAD);
    padded.extend_from_slice(&LEFT_PAD);
    padded.extend_from_slice(&dna);
    let last = *dna.last().expect("au moins 100 nt");
    trits_to_dna(&[0u8; EDGE_PAD], last, &mut padded);

    // 1.5 : une fenêtre toutes les 25 bases.
    let segments: Vec<Vec<u8>> = padded
        .windows(SEGMENT_LEN)
        .step_by(SEGMENT_STEP)
        .enumerate()
        .map(|(segment, window)| strand(segment, window, params.file_id))
        .collect();
    debug_assert_eq!(segments.len(), layout.segments as usize);
    Ok((
        segments,
        EncodeStats {
            segments: layout.segments,
            huffman,
        },
    ))
}

/// Une lecture résolue : sa fenêtre, et ses 100 bases dans le sens du message, les bases
/// douteuses étant absentes.
type Parsed = (usize, Box<[Option<u8>; SEGMENT_LEN]>, usize);

/// Lit une lecture de 117 nt. Rend `None` si elle n'appartient pas à l'archive ou si son index
/// est illisible.
fn parse_read(read: &[u8], file_id: u8, segments: usize) -> Option<Parsed> {
    // Sens de lecture : les bases de repère le disent. Un brin conçu commence par A ou T et finit
    // par C ou G ; lu dans l'autre sens, il commence par G ou C et finit par T ou A. Si une
    // base de repère est erronée, on essaie les deux sens : l'index tranche.
    let forward = matches!(read[0], b'A' | b'T') && matches!(read[STRAND_LEN - 1], b'C' | b'G');
    let reverse = matches!(read[0], b'G' | b'C') && matches!(read[STRAND_LEN - 1], b'A' | b'T');
    if !reverse {
        if let Some(parsed) = parse_strand(read, file_id, segments) {
            return Some(parsed);
        }
    }
    if !forward {
        return parse_strand(&reverse_complement(read), file_id, segments);
    }
    None
}

/// Lit un brin remis dans le sens de sa conception.
fn parse_strand(oriented: &[u8], file_id: u8, segments: usize) -> Option<Parsed> {
    let core = &oriented[1..STRAND_LEN - 1];
    // Deux bases identiques de suite sont impossibles : l'erreur est sur l'une des deux.
    let mut repeated = [false; CORE_LEN - 1];
    let mut located = 0;
    for (position, pair) in core.windows(2).enumerate() {
        if pair[0] == pair[1] {
            repeated[position] = true;
            located += 1;
        }
    }

    // Index : 15 trits, à partir de la dernière base du message.
    let mut previous = base_rank(core[SEGMENT_LEN - 1]).ok()?;
    let mut trits = [0u8; INDEX_TRITS];
    for (trit, &base) in trits.iter_mut().zip(&core[SEGMENT_LEN..]) {
        *trit = trit_between(previous, base).ok()? as u8;
        previous = base_rank(base).ok()?;
    }
    let id = usize::from(trits[0]) * 3 + usize::from(trits[1]);
    if id != usize::from(file_id) {
        return None;
    }
    let mut segment = 0usize;
    for &trit in &trits[ID_TRITS..INDEX_TRITS - 1] {
        segment = segment * 3 + usize::from(trit);
    }
    if segment >= segments
        || trits[INDEX_TRITS - 1] != index_trits(segment, file_id)[INDEX_TRITS - 1]
    {
        return None;
    }

    // Message : les bases voisines d'une répétition, et la première si elle répétait la base de
    // repère, sont écartées du vote.
    let mut bases = [None; SEGMENT_LEN];
    for (position, slot) in bases.iter_mut().enumerate() {
        let doubtful = repeated.get(position).copied().unwrap_or(false)
            || (position > 0 && repeated[position - 1])
            || (position == 0 && oriented[0] == core[0]);
        if !doubtful {
            *slot = Some(core[position]);
        }
    }
    // 1.6 : une fenêtre impaire a été prise en sens inverse.
    if segment % 2 == 1 {
        let mut restored = [None; SEGMENT_LEN];
        for (position, base) in bases.iter().enumerate() {
            restored[SEGMENT_LEN - 1 - position] = base.map(complement);
        }
        bases = restored;
    }
    Some((segment, Box::new(bases), located))
}

/// Décode le corps de l'archive à partir des lectures (manifestes non compris).
pub fn decode(
    reads: &[Vec<u8>],
    layout: &SegmentLayout,
    file_id: u8,
) -> Result<(Vec<u8>, DecodeStats)> {
    let segments = layout.segments as usize;
    let mut stats = DecodeStats::default();

    // 1. Chaque lecture de 117 nt donne une fenêtre et ses bases certaines.
    let sized: Vec<&Vec<u8>> = reads
        .iter()
        .filter(|read| read.len() == STRAND_LEN)
        .collect();
    stats.reads_used = sized.len();
    let parsed: Vec<Parsed> = sized
        .par_iter()
        .filter_map(|read| parse_read(read, file_id, segments))
        .collect();
    stats.uncorrectable = sized.len() - parsed.len();
    stats.corrected = parsed.iter().filter(|(_, _, located)| *located > 0).count();

    // 2. Vote, position par position de l'ADN encodé.
    let mut counts = vec![[0u16; 4]; layout.length];
    let mut seen = vec![false; segments];
    for (segment, bases, _) in &parsed {
        seen[*segment] = true;
        // La fenêtre `segment` commence 75 nt avant le message.
        for (offset, base) in bases.iter().enumerate() {
            // La fenêtre `segment` commence 75 nt avant le message ; elle déborde aussi de
            // l'autre côté.
            let position = segment * SEGMENT_STEP + offset;
            let Some(base) = base else { continue };
            if let Some(count) = position
                .checked_sub(EDGE_PAD)
                .and_then(|position| counts.get_mut(position))
            {
                count[base_rank(*base).expect("base standard")] += 1;
            }
        }
    }
    stats.segments = seen.iter().filter(|&&flag| flag).count();

    // 3. Consensus : la base la plus votée, à condition qu'elle soit seule en tête.
    let mut dna = Vec::with_capacity(layout.length);
    let mut missing = 0usize;
    for count in &counts {
        let mut best = 0usize;
        let mut tied = false;
        for (base, &votes) in count.iter().enumerate() {
            match votes.cmp(&count[best]) {
                std::cmp::Ordering::Greater => {
                    best = base;
                    tied = false;
                }
                std::cmp::Ordering::Equal if base != best => tied = true,
                _ => {}
            }
        }
        if count[best] == 0 {
            missing += 1;
        } else if tied {
            stats.contested += 1;
        } else {
            dna.push(BASES[best]);
        }
    }
    if missing + stats.contested > 0 {
        return Err(DnaError::InsufficientData(format!(
            "{missing} positions du message sans lecture et {} mises à égalité : oligos \
             perdus ou erreurs trop nombreux",
            stats.contested
        )));
    }

    // 4. ADN, trits, puis longueur du message.
    let mut trits = Vec::with_capacity(layout.length);
    let mut previous = 0usize;
    for &base in &dna {
        trits.push(trit_between(previous, base)? as u8);
        previous = base_rank(base)?;
    }
    let (message, length) = trits.split_at(layout.length - LENGTH_TRITS);
    let len_s1 = length
        .iter()
        .fold(0usize, |value, &trit| value * 3 + usize::from(trit));
    if len_s1 > message.len() || message[len_s1..].iter().any(|&trit| trit != 0) {
        return Err(inconsistent("longueur ou bourrage du message"));
    }
    let body = decode_message(&message[..len_s1])?;
    Ok((body, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::rng::SeededRng;
    use crate::sequence::{gc_content, max_homopolymer};

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 17);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    fn dna_of(trits: &[u8], previous: u8) -> Vec<u8> {
        let mut out = Vec::new();
        trits_to_dna(trits, previous, &mut out);
        out
    }

    fn roundtrip(body: &[u8]) -> (Vec<Vec<u8>>, EncodeStats) {
        encode(body, &GoldmanParams::default()).unwrap()
    }

    #[test]
    fn the_table_of_the_article_is_followed() {
        // Figure 1 : pour chaque base précédente, les trois bases des trits 0, 1, 2.
        for (previous, expected) in [
            (b'A', &b"CGT"[..]),
            (b'C', b"GTA"),
            (b'G', b"TAC"),
            (b'T', b"ACG"),
        ] {
            for (trit, &base) in expected.iter().enumerate() {
                assert_eq!(dna_of(&[trit as u8], previous), &[base]);
            }
        }
        // Des trits nuls font tourner le cycle ; des trits quelconques ne répètent jamais.
        assert_eq!(dna_of(&[0; 8], b'A'), b"CGTACGTA");
        let trits: Vec<u8> = pseudo_random(2_000, 3)
            .iter()
            .map(|&byte| byte % 3)
            .collect();
        assert_eq!(max_homopolymer(&dna_of(&trits, b'A')), 1);
    }

    #[test]
    fn the_index_of_the_worked_example_is_reproduced() {
        // Étapes 2.7 à 2.9 de l'information complémentaire : ID = 12, donc fichier 1 × 3 + 2.
        let file_id = 5;
        for segment in 0..3 {
            let index = index_trits(segment, file_id);
            assert_eq!(&index[..ID_TRITS], &[1, 2], "identifiant");
            assert_eq!(index[INDEX_TRITS - 1], 1, "trit de parité P = 1");
        }
        // L'index suit le message à partir de sa dernière base, ici G pour les deux brins.
        assert_eq!(dna_of(&index_trits(0, file_id), b'G'), b"ATACGTACGTACGTC");
        assert_eq!(dna_of(&index_trits(1, file_id), b'G'), b"ATACGTACGTACGAG");
    }

    #[test]
    fn the_fixed_code_is_a_complete_prefix_code() {
        let code = TernaryCode::fixed();
        assert_eq!(code.len_of(&(0..=255u8).collect::<Vec<_>>()), 1_300);
        let bytes: Vec<u8> = (0..=255u8).rev().cycle().take(4_000).collect();
        let mut trits = Vec::new();
        code.encode(&bytes, &mut trits);
        assert_eq!(code.take(&trits, usize::MAX).unwrap().0, bytes);
        // Le mot de 6 trits laissé libre n'est le code d'aucun octet : sur 729 places, les
        // codes de 5 trits en occupent 3 chacune et les codes de 6 trits une.
        let used: u128 = code.counts[1..=6]
            .iter()
            .zip((0..6).rev())
            .map(|(&count, exponent)| u128::from(count) * pow3(exponent))
            .sum();
        assert_eq!(pow3(6) - used, 1);
        assert!(code.take(&[2, 2, 2, 2, 2, 2], 1).is_err());
    }

    #[test]
    fn huffman_lengths_are_those_of_a_ternary_huffman_tree() {
        // Cinq symboles de poids 5, 3, 2, 1, 1 : (1, 1, 2) est fusionné, puis (3, 4, 5). Les
        // deux symboles les plus fréquents sont à profondeur 1, les autres à 2.
        let mut counts = [0u64; 256];
        for (symbol, count) in [(0u8, 5u64), (1, 3), (2, 2), (3, 1), (4, 1)] {
            counts[usize::from(symbol)] = count;
        }
        let lengths = huffman_lengths(&counts).unwrap();
        assert_eq!(&lengths[..6], &[1, 1, 2, 2, 2, 0]);
        assert_eq!(
            TernaryCode::from_lengths(&lengths).unwrap().len_of(&[0, 1]),
            2
        );
        // Neuf symboles équiprobables : un arbre ternaire de profondeur 2.
        let mut counts = [0u64; 256];
        counts[..9].fill(7);
        assert!(huffman_lengths(&counts).unwrap()[..9]
            .iter()
            .all(|&len| len == 2));
        // 256 octets équiprobables, plus une feuille nulle : 236 codes de 5 trits et 21 de 6
        // au plus, comme le code fixe (1300 ou 1301 trits pour les 256 octets).
        let lengths = huffman_lengths(&[7u64; 256]).unwrap();
        assert!(lengths.iter().all(|&len| len == 5 || len == 6));
        let code = TernaryCode::from_lengths(&lengths).unwrap();
        let all: Vec<u8> = (0..=255u8).collect();
        assert!(matches!(code.len_of(&all), 1_300 | 1_301));
        let mut trits = Vec::new();
        code.encode(&all, &mut trits);
        assert_eq!(code.take(&trits, usize::MAX).unwrap().0, all);
        // Un seul octet présent : un trit par octet. Aucun : rien.
        let mut counts = [0u64; 256];
        counts[42] = 100;
        assert_eq!(huffman_lengths(&counts).unwrap()[42], 1);
        assert_eq!(huffman_lengths(&[0u64; 256]).unwrap(), [0u8; 256]);
    }

    #[test]
    fn the_huffman_code_is_used_only_when_it_pays_off() {
        // Octets au hasard : le code fixe, sans table.
        let random = pseudo_random(10_000, 1);
        let (huffman, trits) = encode_message(&random).unwrap();
        assert!(!huffman);
        assert_eq!(trits.len(), 5 + TernaryCode::fixed().len_of(&random));
        // Texte : le code de Huffman, table comprise, est plus court.
        let text: Vec<u8> = b"le renard brun saute par-dessus le chien paresseux. "
            .iter()
            .cycle()
            .take(10_000)
            .copied()
            .collect();
        let (used, short) = encode_message(&text).unwrap();
        assert!(used);
        assert!(
            short.len() * 12 < trits.len() * 10,
            "{} {}",
            short.len(),
            trits.len()
        );
        assert_eq!(decode_message(&short).unwrap(), text);
        assert_eq!(decode_message(&trits).unwrap(), random);
        // Un seul octet, très répété, et un corps vide.
        let (used, one) = encode_message(&vec![b'@'; 3_000]).unwrap();
        assert!(used);
        assert_eq!(one.len(), TABLE_TRITS + 3_000);
        assert_eq!(decode_message(&one).unwrap(), vec![b'@'; 3_000]);
        assert_eq!(encode_message(&[]).unwrap().1.len(), 5);
    }

    #[test]
    fn a_damaged_code_or_stream_is_refused() {
        let (_, trits) = encode_message(b"bonjour le monde").unwrap();
        let mut damaged = trits.clone();
        damaged[6] = 5;
        assert!(decode_message(&damaged).is_err());
        let mut damaged = trits.clone();
        damaged.truncate(damaged.len() - 1);
        assert!(decode_message(&damaged).is_err());
        // Table absurde : 256 codes d'un trit.
        let fixed = TernaryCode::fixed();
        let mut absurd = Vec::new();
        fixed.encode(&[MODE_HUFFMAN], &mut absurd);
        fixed.encode(&[1u8; 256], &mut absurd);
        assert!(decode_message(&absurd).is_err());
        let mut lengths = [0u8; 256];
        lengths[..4].fill(1);
        assert!(TernaryCode::from_lengths(&lengths).is_err());
        let mut lengths = [0u8; 256];
        lengths[..3].fill(1);
        assert!(TernaryCode::from_lengths(&lengths).is_ok());
    }

    #[test]
    fn layout_follows_the_article() {
        // Exemple 2.3 : 92 trits, 13 de bourrage, 20 de longueur : 125.
        let layout = SegmentLayout::for_message(92).unwrap();
        assert_eq!(layout.length, 125);
        // 125 / 25 + 3 = 8 fenêtres avec le bourrage des extrémités ; l'article, qui n'en a
        // pas, en comptait 2.
        assert_eq!(layout.segments, 8);
        assert_eq!(SegmentLayout::from_manifest(8, 92).unwrap(), layout);
        assert_eq!(SegmentLayout::for_message(0).unwrap().length, 100);
        assert_eq!(SegmentLayout::for_message(5).unwrap().length, 100);
        assert_eq!(SegmentLayout::for_message(80).unwrap().length, 100);
        assert_eq!(SegmentLayout::for_message(81).unwrap().length, 125);
        assert_eq!(
            SegmentLayout::for_message(usize::MAX)
                .unwrap_err()
                .to_string(),
            too_large().to_string()
        );
        // 531 441 fenêtres : 531 438 de message, 25 nt chacune, et 3 de bourrage.
        let largest = SegmentLayout::for_message(13_285_930).unwrap();
        assert_eq!(largest.segments as usize, MAX_SEGMENTS);
        assert!(SegmentLayout::for_message(13_285_931).is_err());
        for absurd in [0, 6, MAX_SEGMENTS as u32 + 1, u32::MAX] {
            assert!(SegmentLayout::from_manifest(absurd, 0).is_err());
        }
        assert!(SegmentLayout::from_manifest(8, 126).is_err());
        assert!(SegmentLayout::from_manifest(8, 125).is_ok());
    }

    #[test]
    fn roundtrip_and_every_strand_has_the_shape_of_the_article() {
        let params = GoldmanParams::default();
        for size in [0usize, 1, 60, 1_000, 5_000, 100_000] {
            let body = pseudo_random(size, size as u64 + 1);
            let (strands, stats) = encode(&body, &params).unwrap();
            let layout = SegmentLayout::from_manifest(stats.segments, body.len() as u64).unwrap();
            assert_eq!(strands.len(), stats.segments as usize);
            let mut gc = 0.0;
            for strand in &strands {
                assert_eq!(strand.len(), STRAND_LEN);
                assert_eq!(max_homopolymer(strand), 1, "jamais deux bases égales");
                assert!(matches!(strand[0], b'A' | b'T') && matches!(strand[116], b'C' | b'G'));
                gc += gc_content(strand);
            }
            // Le GC n'est pas garanti par brin, mais il est de 50 % en moyenne dès que le
            // message domine le bourrage.
            if size >= 5_000 {
                assert!(
                    (gc / strands.len() as f64 - 0.5).abs() < 0.02,
                    "taille {size}"
                );
            }
            let (decoded, decode_stats) = decode(&strands, &layout, params.file_id).unwrap();
            assert_eq!(decoded, body, "taille {size}");
            assert_eq!(decode_stats.uncorrectable, 0);
            assert_eq!(decode_stats.segments, strands.len());
        }
    }

    #[test]
    fn every_position_is_read_in_four_strands() {
        let body = pseudo_random(3_000, 7);
        let (strands, stats) = roundtrip(&body);
        let layout = SegmentLayout::from_manifest(stats.segments, body.len() as u64).unwrap();
        let mut coverage = vec![0usize; layout.length];
        for strand in &strands {
            let (segment, _bases, _) = parse_read(strand, 0, stats.segments as usize).unwrap();
            for offset in 0..SEGMENT_LEN {
                let position = segment * SEGMENT_STEP + offset;
                if (EDGE_PAD..EDGE_PAD + layout.length).contains(&position) {
                    coverage[position - EDGE_PAD] += 1;
                }
            }
        }
        assert!(coverage.iter().all(|&times| times == 4), "{coverage:?}");
    }

    #[test]
    fn the_output_does_not_depend_on_the_order_nor_on_the_reading_direction() {
        let body = pseudo_random(4_000, 11);
        let (strands, stats) = roundtrip(&body);
        let layout = SegmentLayout::from_manifest(stats.segments, body.len() as u64).unwrap();
        let mut shuffled = strands.clone();
        let mut rng = SeededRng::new(1, 1);
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, rng.below(i as u64 + 1) as usize);
        }
        // La moitié des brins sont lus depuis l'autre bout.
        for strand in shuffled.iter_mut().step_by(2) {
            *strand = reverse_complement(strand);
        }
        let (decoded, _) = decode(&shuffled, &layout, 0).unwrap();
        assert_eq!(decoded, body);
    }

    #[test]
    fn an_error_never_leaves_two_identical_bases_unnoticed() {
        // Toute substitution dans l'index (15 dernières bases) est détectée, et toute
        // substitution du message qui crée une répétition est localisée.
        let body = pseudo_random(2_000, 13);
        let (strands, stats) = roundtrip(&body);
        let segments = stats.segments as usize;
        for position in CORE_LEN..STRAND_LEN - 1 {
            for &replacement in b"ACGT" {
                let mut damaged = strands[3].clone();
                if damaged[position] == replacement {
                    continue;
                }
                damaged[position] = replacement;
                assert!(
                    parse_read(&damaged, 0, segments).is_none(),
                    "position {position}"
                );
            }
        }
        let mut located = 0;
        let mut kept = 0;
        for strand in &strands {
            for position in 1..CORE_LEN {
                for &replacement in b"ACGT" {
                    if strand[position] == replacement || strand[position - 1] != replacement {
                        continue;
                    }
                    let mut damaged = strand.clone();
                    damaged[position] = replacement;
                    let Some((_, bases, found)) = parse_read(&damaged, 0, segments) else {
                        continue;
                    };
                    kept += 1;
                    located += found;
                    // La base fausse et ses voisines sont écartées du vote (trois bases
                    // si elle crée une suite de trois).
                    let erased: Vec<usize> = bases
                        .iter()
                        .enumerate()
                        .filter_map(|(offset, base)| base.is_none().then_some(offset))
                        .collect();
                    assert!(erased.len() <= 3, "{erased:?}");
                }
            }
        }
        // Chaque lecture conservée a vu sa répétition, et parfois deux (une suite de trois).
        assert!(kept > 1_000);
        assert!(located >= kept, "{kept} {located}");
    }

    #[test]
    fn substitutions_are_outvoted_and_ties_are_announced() {
        let body = pseudo_random(5_000, 17);
        let (strands, stats) = roundtrip(&body);
        let layout = SegmentLayout::from_manifest(stats.segments, body.len() as u64).unwrap();

        // Une substitution par brin, dans le message (hors dernière base, qui porte sur
        // l'index). À couverture 1, chaque base a quatre lectures : l'erreur est toujours
        // minoritaire.
        let damaged: Vec<Vec<u8>> = strands
            .iter()
            .enumerate()
            .map(|(rank, strand)| {
                let mut damaged = strand.clone();
                let position = 1 + (rank * 7) % (SEGMENT_LEN - 1);
                damaged[position] = match damaged[position] {
                    b'A' => b'C',
                    _ => b'A',
                };
                damaged
            })
            .collect();
        let (decoded, decode_stats) = decode(&damaged, &layout, 0).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(decode_stats.uncorrectable, 0);
        assert!(
            decode_stats.corrected > 0,
            "des répétitions ont été repérées"
        );

        // Trois des quatre fenêtres qui couvrent la première base manquent : une lecture
        // subsiste, le vote passe. Quatre : la position est sans lecture, l'échec est annoncé.
        let kept: Vec<Vec<u8>> = strands.iter().skip(3).cloned().collect();
        assert_eq!(decode(&kept, &layout, 0).unwrap().0, body);
        let kept: Vec<Vec<u8>> = strands.iter().skip(4).cloned().collect();
        let err = decode(&kept, &layout, 0).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");

        // Deux lectures fautives et concordantes contre deux exactes : la première base du
        // message est mise à égalité (fenêtre 0, en ordre ; fenêtre 1, en sens inverse).
        let mut tied = strands.clone();
        let exact = tied[0][1 + EDGE_PAD];
        let wrong = b"ACGT"
            .iter()
            .copied()
            .find(|&base| {
                base != exact && base != tied[0][EDGE_PAD] && base != tied[0][2 + EDGE_PAD]
            })
            .expect("une base qui ne répète pas ses voisines");
        tied[0][1 + EDGE_PAD] = wrong;
        tied[1][1 + SEGMENT_LEN - 1 - (EDGE_PAD - SEGMENT_STEP)] = complement(wrong);
        let err = decode(&tied, &layout, 0).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");

        // Des lectures d'un autre fichier ne comptent pas.
        let other = encode(&body, &GoldmanParams { file_id: 7 }).unwrap().0;
        let mut mixed = strands.clone();
        mixed.extend(other);
        let (decoded, decode_stats) = decode(&mixed, &layout, 0).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(decode_stats.uncorrectable, strands.len());
        // La même archive lue sous un autre identifiant : rien.
        assert!(decode(&strands, &layout, 7).is_err());
    }

    #[test]
    fn reads_of_the_wrong_length_are_ignored() {
        let body = pseudo_random(3_000, 19);
        let (strands, stats) = roundtrip(&body);
        let layout = SegmentLayout::from_manifest(stats.segments, body.len() as u64).unwrap();
        let mut reads = strands.clone();
        reads.push(vec![b'A'; STRAND_LEN - 1]);
        reads.push(vec![b'A'; STRAND_LEN + 4]);
        reads.push(Vec::new());
        let (decoded, decode_stats) = decode(&reads, &layout, 0).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(decode_stats.reads_used, strands.len());
        assert!(decode(&[], &layout, 0).is_err());
    }

    #[test]
    fn parameters_are_checked() {
        assert!(GoldmanParams { file_id: 8 }.validate().is_ok());
        assert!(GoldmanParams { file_id: 9 }.validate().is_err());
    }
}
