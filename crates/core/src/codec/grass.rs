//! Schéma « grass » : Reed-Solomon sur le corps à 47 éléments, chaque symbole écrit en trois
//! nucléotides (Grass et al., 2015).
//!
//! L'article encode l'information en caractères de GF(47), corrigés par du Reed-Solomon
//! entrelacé par blocs de 713 brins × 39 caractères, et écrit chaque caractère en un trimère
//! choisi pour qu'aucune suite de plus de trois bases identiques n'apparaisse. Le corps à 47
//! éléments vient des trimères : il y a 36 trimères sans deux bases adjacentes égales et 12
//! trimères terminés par un doublet, soit 48 trimères utilisables, et 47 est le plus grand
//! premier de cette taille.
//!
//! | index, 4 symboles | nonce, 1 symbole | charge utile, P symboles | parité interne, 39 - 5 - P |
//!
//! Chaque symbole fait 3 nt : le brin en compte 39, soit 117 nt. Deux niveaux de code, comme
//! dans l'article :
//!
//! - le **code interne**, brin par brin : Reed-Solomon sur GF(47), 39 symboles dont 14 de parité
//!   par défaut ; il corrige jusqu'à sept symboles erronés par lecture ;
//! - le **code externe**, bloc par bloc : un seul Reed-Solomon sur GF(47²), sur les 713 brins
//!   du bloc — 621 de données et 92 de parité, soit 13 % ; il répare jusqu'à 92 brins manquants
//!   par bloc.
//!
//! Le nonce sert au criblage : le nonce, la charge utile et la parité sont masquées par un flux
//! dérivé du nonce, et l'encodeur essaie les 47 nonces jusqu'à obtenir un brin qui respecte les
//! contraintes biochimiques. Un trimère hors du jeu des 47 signale une erreur à sa position : le
//! décodeur en fait un effacement, que le code interne rétablit sans compter dans ses erreurs.
//!
//! L'adressage logique (l'index) et le code externe sont des ajouts du format v2 : l'article
//! adressait chaque brin par ses amorces. Le nombre de symboles de parité interne, 14 par
//! défaut, se règle.

use crate::codec::rng::SeededRng;
use crate::constraints::DnaConstraints;
use crate::error::{DnaError, Result};
use crate::rs47::gf2209::Gf2209;
use crate::rs47::gf47::Gf47;
use crate::rs47::{Field, Rs2209Code, Rs47Code};
use crate::sequence::{base_to_bits, BASES};
use rayon::prelude::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Longueur d'un brin, en nucléotides.
pub const STRAND_LEN: usize = 117;
/// Nucléotides par symbole.
pub const SYMBOL_LEN: usize = 3;
/// Symboles d'un brin.
pub const SYMBOLS: usize = 39;
/// Symboles d'index : le nombre du brin, en base 47.
pub const INDEX_SYMBOLS: usize = 4;
/// Symboles derrière l'index : le nonce, la charge utile et la parité interne.
pub const TAIL_SYMBOLS: usize = SYMBOLS - INDEX_SYMBOLS;
/// Brins d'un bloc : 713 = 23 × 31.
pub const BLOCK: usize = 713;
/// Brins de parité externe par bloc : 92 pour 621 de données, soit 13 %.
pub const BLOCK_PARITY: usize = 92;
/// Brins de données par bloc.
pub const BLOCK_DATA: usize = BLOCK - BLOCK_PARITY;
/// Symboles de parité interne par brin, par défaut.
pub const DEFAULT_PARITY: usize = 14;
pub const MIN_PARITY: usize = 2;
pub const MAX_PARITY: usize = 30;
/// Nombre de brins que l'index permet : 47^4.
pub const MAX_STRANDS: usize = 47 * 47 * 47 * 47;
/// Nombre de nonces distincts.
pub const NONCES: u64 = 47;

const MASK_STREAM: u64 = 0x0000_0000_0000_0014;

/// Masque fixe de l'index. Sans lui, les brins commenceraient tous par les mêmes trimères,
/// ceux des chiffres de petits nombres.
const INDEX_MASK: [u8; INDEX_SYMBOLS] = [23, 7, 41, 15];

/// Les 47 trimères, dans l'ordre des symboles 0 à 46, et le symbole de chacun des 64 trimères
/// possibles (−1 pour les 17 écartés).
const TABLES: ([[u8; SYMBOL_LEN]; 47], [i8; 64]) = build_tables();

const fn build_tables() -> ([[u8; SYMBOL_LEN]; 47], [i8; 64]) {
    let mut trimers = [[0u8; SYMBOL_LEN]; 47];
    let mut symbol_of = [-1i8; 64];
    let mut count = 0usize;
    // Les 36 trimères sans deux bases adjacentes égales, puis 11 des 12 trimères terminés par un
    // doublet : `CAA` est écarté, pour que le jeu reste équilibré en GC.
    let mut pass = 0;
    while pass < 2 {
        let mut code = 0usize;
        while code < 64 {
            let bases = [BASES[code >> 4], BASES[(code >> 2) & 3], BASES[code & 3]];
            let kept = if pass == 0 {
                bases[0] != bases[1] && bases[1] != bases[2]
            } else {
                bases[0] != bases[1]
                    && bases[1] == bases[2]
                    && !(bases[0] == b'C' && bases[1] == b'A')
            };
            if kept {
                trimers[count] = bases;
                symbol_of[code] = count as i8;
                count += 1;
            }
            code += 1;
        }
        pass += 1;
    }
    (trimers, symbol_of)
}

/// Paramètres du schéma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrassParams {
    /// Symboles de parité interne par brin, pairs, de 2 à 30. Un brin résiste à la moitié de
    /// symboles erronés ; sa charge utile diminue d'autant. La parité est paire parce que le
    /// code externe prend les symboles de charge utile deux par deux.
    pub parity: usize,
}

impl Default for GrassParams {
    fn default() -> Self {
        Self {
            parity: DEFAULT_PARITY,
        }
    }
}

impl GrassParams {
    /// Vérifie la cohérence des paramètres.
    pub fn validate(&self) -> Result<()> {
        if !(MIN_PARITY..=MAX_PARITY).contains(&self.parity) || !self.parity.is_multiple_of(2) {
            return Err(DnaError::InvalidConfig(format!(
                "parité interne du schéma grass invalide: {} (attendu : un nombre pair de {} à {})",
                self.parity, MIN_PARITY, MAX_PARITY
            )));
        }
        Ok(())
    }
}

/// Répartition d'un corps en brins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrassLayout {
    /// Symboles de parité interne par brin.
    pub parity: usize,
    /// Symboles de charge utile par brin.
    pub payload: usize,
    /// Octets de corps portés par un brin de données.
    pub bytes_per_strand: usize,
    pub data_strands: usize,
    pub blocks: usize,
    /// Taille de l'espace des index : 713 par bloc, y compris les places inutilisées du dernier.
    pub total_strands: usize,
}

fn too_large() -> DnaError {
    DnaError::Encoding(format!(
        "fichier trop volumineux pour le schéma grass (limite: {MAX_STRANDS} brins)"
    ))
}

/// Grand entier de 256 bits, en membres de 32 bits, le plus significatif en tête. Le plus grand
/// nombre manipulé, 47^32, en utilise 178.
type Wide = [u32; 8];

/// Multiplie par `factor` et ajoute `addend`.
fn wide_mul_add(value: &mut Wide, factor: u32, addend: u32) {
    let mut carry = u64::from(addend);
    for limb in value.iter_mut().rev() {
        let product = u64::from(*limb) * u64::from(factor) + carry;
        *limb = product as u32;
        carry = product >> 32;
    }
    debug_assert_eq!(carry, 0, "débordement du grand entier");
}

/// Divise par `divisor` et rend le reste.
fn wide_div_rem(value: &mut Wide, divisor: u32) -> u32 {
    let mut remainder = 0u64;
    for limb in value.iter_mut() {
        let current = remainder << 32 | u64::from(*limb);
        *limb = (current / u64::from(divisor)) as u32;
        remainder = current % u64::from(divisor);
    }
    remainder as u32
}

/// Vrai si `symbols` symboles de GF(47) portent `bytes` octets : 256^octets <= 47^symboles.
fn fits(bytes: usize, symbols: usize) -> bool {
    let mut capacity: Wide = [0; 8];
    capacity[7] = 1;
    for _ in 0..symbols {
        wide_mul_add(&mut capacity, 47, 0);
    }
    let bits = 8 * bytes;
    let mut value: Wide = [0; 8];
    value[7 - bits / 32] = 1 << (bits % 32);
    value <= capacity
}

/// Octets portés par `symbols` symboles.
fn bytes_carried(symbols: usize) -> usize {
    (1..).take_while(|&bytes| fits(bytes, symbols)).count()
}

/// Écrit `bytes` (poids fort en tête) en `count` symboles, poids fort en tête.
fn bytes_to_symbols(bytes: &[u8], count: usize) -> Vec<u8> {
    let mut value: Wide = [0; 8];
    for &byte in bytes {
        wide_mul_add(&mut value, 256, u32::from(byte));
    }
    let mut symbols = vec![0u8; count];
    for slot in symbols.iter_mut().rev() {
        *slot = wide_div_rem(&mut value, 47) as u8;
    }
    debug_assert_eq!(value, [0; 8], "trop d'octets pour ce nombre de symboles");
    symbols
}

/// Lit `len` octets (poids fort en tête) dans des symboles, poids fort en tête.
fn symbols_to_bytes(symbols: &[Gf47], len: usize) -> Vec<u8> {
    let mut value: Wide = [0; 8];
    for symbol in symbols {
        wide_mul_add(&mut value, 47, u32::from(symbol.0));
    }
    let mut bytes = vec![0u8; len];
    for slot in bytes.iter_mut().rev() {
        *slot = wide_div_rem(&mut value, 256) as u8;
    }
    bytes
}

impl GrassLayout {
    /// Répartition pour un corps de `body_len` octets, si le format peut le contenir.
    pub fn for_body(body_len: usize, parity: usize) -> Result<Self> {
        GrassParams { parity }.validate().map_err(|_| {
            DnaError::InvalidConfig(format!("parité interne du schéma grass invalide: {parity}"))
        })?;
        let payload = TAIL_SYMBOLS - 1 - parity;
        let bytes_per_strand = bytes_carried(payload);
        let data_strands = body_len.div_ceil(bytes_per_strand);
        let blocks = data_strands.div_ceil(BLOCK_DATA);
        let Some(total_strands) = blocks
            .checked_mul(BLOCK)
            .filter(|&total| total <= MAX_STRANDS)
        else {
            return Err(too_large());
        };
        Ok(Self {
            parity,
            payload,
            bytes_per_strand,
            data_strands,
            blocks,
            total_strands,
        })
    }

    /// Nombre de brins de données du bloc `block`.
    pub fn data_in_block(&self, block: usize) -> usize {
        self.data_strands
            .saturating_sub(block * BLOCK_DATA)
            .min(BLOCK_DATA)
    }

    /// Vrai si le brin `position` du bloc `block` existe.
    pub fn strand_exists(&self, block: usize, position: usize) -> bool {
        position >= BLOCK_DATA || position < self.data_in_block(block)
    }

    /// Le code externe, commun à tous les blocs : 713 symboles dont 621 de donnée, les places
    /// inutilisées d'un dernier bloc incomplet comptant pour zéro.
    fn outer_code() -> Rs2209Code {
        Rs2209Code::new(BLOCK, BLOCK_DATA).expect("713 symboles, sur un corps de 2 209")
    }
}

/// Bilan d'un encodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub data_strands: usize,
    pub parity_strands: usize,
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

/// Flux de masquage, dérivé du nonce, pour les symboles derrière l'index.
fn mask(nonce: u8, out: &mut [u8; TAIL_SYMBOLS]) {
    let mut rng = SeededRng::new(u64::from(nonce), MASK_STREAM);
    for slot in out.iter_mut() {
        *slot = rng.below(NONCES) as u8;
    }
}

/// Chiffres d'un nombre en base 47, poids fort en tête.
fn digits(mut value: usize) -> [u8; INDEX_SYMBOLS] {
    let mut out = [0u8; INDEX_SYMBOLS];
    for slot in out.iter_mut().rev() {
        *slot = (value % 47) as u8;
        value /= 47;
    }
    out
}

/// Nombre écrit par des chiffres en base 47, poids fort en tête.
fn number(digits: &[u8]) -> usize {
    digits
        .iter()
        .fold(0usize, |value, &digit| value * 47 + usize::from(digit))
}

/// Rang d'un trimère parmi les 64 possibles.
fn trimer_code(trimer: &[u8]) -> usize {
    usize::from(base_to_bits(trimer[0]).expect("base standard")) << 4
        | usize::from(base_to_bits(trimer[1]).expect("base standard")) << 2
        | usize::from(base_to_bits(trimer[2]).expect("base standard"))
}

/// Symboles du brin, sur le fil : l'index masqué par le masque fixe, le nonce en clair, le
/// reste masqué par le flux du nonce. Les rangs vont de 0 à 46.
fn wire_symbols(
    index: usize,
    nonce: u8,
    payload: &[Gf47],
    inner: &Rs47Code,
    stream: &mut [u8; TAIL_SYMBOLS],
) -> Result<[u8; SYMBOLS]> {
    let mut codeword = [Gf47::new(0); SYMBOLS];
    let head = digits(index);
    for (position, &digit) in head.iter().enumerate() {
        codeword[position] = Gf47::new(digit);
    }
    codeword[INDEX_SYMBOLS] = Gf47::new(nonce);
    codeword[INDEX_SYMBOLS + 1..INDEX_SYMBOLS + 1 + payload.len()].copy_from_slice(payload);
    // La parité se calcule sur le mot de code, avant tout masquage.
    inner.add_parity(&mut codeword)?;
    mask(nonce, stream);
    let mut wire = [0u8; SYMBOLS];
    for position in 0..INDEX_SYMBOLS {
        wire[position] = (head[position] + INDEX_MASK[position]) % 47;
    }
    wire[INDEX_SYMBOLS] = nonce;
    for (offset, symbol) in codeword[INDEX_SYMBOLS + 1..].iter().enumerate() {
        wire[INDEX_SYMBOLS + 1 + offset] =
            (usize::from(symbol.0) + usize::from(stream[offset])) as u8 % 47;
    }
    Ok(wire)
}

/// Construit le brin d'index et de charge utile donnés, en essayant les nonces jusqu'à ce que
/// le brin respecte les contraintes.
fn build_strand(
    index: usize,
    payload: &[Gf47],
    inner: &Rs47Code,
    constraints: &DnaConstraints,
    nonces_tried: &mut u64,
) -> Result<Vec<u8>> {
    let mut stream = [0u8; TAIL_SYMBOLS];
    let mut dna = Vec::with_capacity(STRAND_LEN);
    for nonce in 0..NONCES as u8 {
        *nonces_tried += 1;
        let wire = wire_symbols(index, nonce, payload, inner, &mut stream)?;
        dna.clear();
        for &symbol in &wire {
            dna.extend_from_slice(&TABLES.0[usize::from(symbol)]);
        }
        if constraints.satisfied(&dna) {
            return Ok(dna);
        }
    }
    Err(DnaError::Encoding(format!(
        "aucun nonce ne donne un brin conforme pour l'index {index}: contraintes trop strictes"
    )))
}

/// Deux symboles de charge utile consécutifs, vus comme un symbole de GF(47²).
fn pair(strand: &[Gf47], column: usize) -> Gf2209 {
    Gf2209 {
        a: strand[2 * column].0,
        b: strand[2 * column + 1].0,
    }
}

/// Charge utile d'un brin, données ou parité externe.
fn payload_of<'a>(
    index: usize,
    payload: usize,
    data_symbols: &'a [Gf47],
    parity_symbols: &'a [Gf47],
) -> &'a [Gf47] {
    let block = index / BLOCK;
    let position = index % BLOCK;
    if position < BLOCK_DATA {
        &data_symbols[(block * BLOCK_DATA + position) * payload..][..payload]
    } else {
        let rank = block * BLOCK_PARITY + position - BLOCK_DATA;
        &parity_symbols[rank * payload..][..payload]
    }
}

/// Encode le corps d'une archive en brins (manifestes non compris).
pub fn encode(
    body: &[u8],
    params: &GrassParams,
    constraints: &DnaConstraints,
) -> Result<(Vec<Vec<u8>>, EncodeStats)> {
    let layout = GrassLayout::for_body(body.len(), params.parity)?;
    if layout.data_strands == 0 {
        return Ok((Vec::new(), EncodeStats::default()));
    }
    let payload = layout.payload;
    let columns = payload / 2;
    let inner = Rs47Code::new(SYMBOLS, SYMBOLS - params.parity)?;

    // 1. Charge utile des brins de données, en symboles.
    let mut padded = body.to_vec();
    padded.resize(layout.data_strands * layout.bytes_per_strand, 0);
    let data_symbols: Vec<Gf47> = padded
        .chunks(layout.bytes_per_strand)
        .flat_map(|bytes| bytes_to_symbols(bytes, payload))
        .map(Gf47::new)
        .collect();

    // 2. Code externe : dans chaque bloc, pour chaque colonne de GF(47²) — deux symboles de
    //    charge utile —, la parité des données du bloc. Les places que le dernier bloc
    //    n'occupe pas comptent pour zéro : c'est le même code pour tous les blocs.
    let outer = GrassLayout::outer_code();
    let mut parity_symbols = vec![Gf47::new(0); layout.blocks * BLOCK_PARITY * payload];
    let mut column = vec![Gf2209::zero(); BLOCK];
    for block in 0..layout.blocks {
        let count = layout.data_in_block(block);
        for column_index in 0..columns {
            column.fill(Gf2209::zero());
            for (value, i) in column.iter_mut().zip(0..count) {
                *value = pair(
                    &data_symbols[(block * BLOCK_DATA + i) * payload..][..payload],
                    column_index,
                );
            }
            outer.add_parity(&mut column)?;
            for (rank, value) in column[BLOCK_DATA..].iter().enumerate() {
                let base = (block * BLOCK_PARITY + rank) * payload + 2 * column_index;
                parity_symbols[base] = Gf47::new(value.a);
                parity_symbols[base + 1] = Gf47::new(value.b);
            }
        }
    }

    // 3. Un brin par index : code interne, puis criblage par le nonce.
    //
    // Chaque brin a 47 candidats. Le risque qu'aucun ne convienne, cumulé sur l'archive, doit
    // rester négligeable ; sinon les contraintes sont refusées d'emblée. La limite des suites
    // identiques ne pèse pas : les trimères la tiennent par construction, dès qu'elle atteint 3.
    let failure = (1.0 - constraints.gc_pass_probability(STRAND_LEN)).powi(NONCES as i32)
        * layout.total_strands as f64;
    if failure > 1e-6 {
        return Err(DnaError::Encoding(
            constraints.too_strict_message(STRAND_LEN),
        ));
    }
    let nonces_tried = AtomicU64::new(0);
    let oligos: Vec<Vec<u8>> = (0..layout.total_strands)
        .into_par_iter()
        .filter(|&index| layout.strand_exists(index / BLOCK, index % BLOCK))
        .map(|index| {
            let mut local = 0u64;
            let outcome = build_strand(
                index,
                payload_of(index, payload, &data_symbols, &parity_symbols),
                &inner,
                constraints,
                &mut local,
            );
            nonces_tried.fetch_add(local, Ordering::Relaxed);
            outcome
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((
        oligos,
        EncodeStats {
            data_strands: layout.data_strands,
            parity_strands: layout.blocks * BLOCK_PARITY,
            nonces_tried: nonces_tried.load(Ordering::Relaxed),
        },
    ))
}

/// Une lecture résolue : l'index, la charge utile et le nombre de symboles corrigés.
type Parsed = (usize, Box<[Gf47]>, usize);

/// Décode le mot de code interne pour un nonce donné.
fn parse_with(
    nonce: u8,
    wire: &[u8; SYMBOLS],
    erased: &[usize],
    inner: &Rs47Code,
    total: usize,
) -> Result<Parsed> {
    let mut codeword = [Gf47::new(0); SYMBOLS];
    // L'index retranche son masque fixe, le nonce est remplacé par celui que l'on tente, et le
    // reste retranche le flux de ce nonce. Chaque opération est une bijection par symbole : les
    // positions des effacements ne bougent pas.
    for position in 0..INDEX_SYMBOLS {
        codeword[position] = Gf47::new((wire[position] + 47 - INDEX_MASK[position]) % 47);
    }
    codeword[INDEX_SYMBOLS] = Gf47::new(nonce);
    let mut stream = [0u8; TAIL_SYMBOLS];
    mask(nonce, &mut stream);
    for (offset, symbol) in codeword[INDEX_SYMBOLS + 1..].iter_mut().enumerate() {
        *symbol = Gf47::new(
            (usize::from(wire[INDEX_SYMBOLS + 1 + offset]) + 47 - usize::from(stream[offset]))
                as u8
                % 47,
        );
    }
    let fixed = inner.correct(&mut codeword, erased)?;
    // Seul le vrai nonce donne un mot de code qui le confirme.
    if codeword[INDEX_SYMBOLS].0 != nonce {
        return Err(DnaError::Correction("nonce contredit".to_string()));
    }
    let head: Vec<u8> = codeword[..INDEX_SYMBOLS]
        .iter()
        .map(|symbol| symbol.0)
        .collect();
    let index = number(&head);
    if index >= total {
        return Err(DnaError::Correction("index hors archive".to_string()));
    }
    let payload = Box::from(&codeword[INDEX_SYMBOLS + 1..inner.data_len()]);
    Ok((index, payload, fixed))
}

/// Décode une lecture : renvoie l'index, la charge utile et le nombre de symboles corrigés.
///
/// Le nonce est lu en clair. S'il est erroné, le masque est faux : on essaie alors les autres
/// nonces, et l'on n'accepte que celui que le code interne confirme.
fn parse_read(read: &[u8], inner: &Rs47Code, total: usize) -> Result<Parsed> {
    let mut wire = [0u8; SYMBOLS];
    let mut erased: Vec<usize> = Vec::new();
    for position in 0..SYMBOLS {
        match TABLES.1[trimer_code(&read[SYMBOL_LEN * position..])] {
            -1 => erased.push(position),
            symbol => wire[position] = symbol as u8,
        }
    }
    let nonce = if erased.contains(&INDEX_SYMBOLS) {
        None
    } else {
        Some(wire[INDEX_SYMBOLS])
    };
    if let Some(nonce) = nonce {
        if let Ok(parsed) = parse_with(nonce, &wire, &erased, inner, total) {
            return Ok(parsed);
        }
    }
    (0..NONCES as u8)
        .filter(|&candidate| Some(candidate) != nonce)
        .find_map(|candidate| parse_with(candidate, &wire, &erased, inner, total).ok())
        .ok_or_else(|| DnaError::Correction("lecture de brin irrécupérable".to_string()))
}

struct IndexEntry {
    payload: Box<[Gf47]>,
    votes: u32,
    others: u32,
}

/// Contrôle ou répare un bloc avec le code externe, colonne par colonne.
///
/// Un brin manquant est un effacement et un symbole faux une erreur : chaque colonne admet
/// `2 · erreurs + effacements <= 92`. Rend le nombre de brins de données restitués ou corrigés ;
/// 0 pour un bloc sain.
fn repair_block(
    block: usize,
    slots: &mut [Option<Box<[Gf47]>>],
    layout: &GrassLayout,
    outer: &Rs2209Code,
    payload: usize,
) -> Result<usize> {
    let count = layout.data_in_block(block);
    let length = BLOCK;
    // Les places que le dernier bloc n'occupe pas comptent pour zéro : elles ne manquent pas.
    let missing: Vec<usize> = (0..length)
        .filter(|&position| layout.strand_exists(block, position) && slots[position].is_none())
        .collect();
    let missing_data = missing.iter().filter(|&&position| position < count).count();
    if missing_data > BLOCK_PARITY {
        return Err(DnaError::InsufficientData(format!(
            "bloc {block} : {missing_data} brins de données manquants pour {BLOCK_PARITY} \
             brins de parité"
        )));
    }

    // Écrit dans `column` les valeurs d'une colonne de GF(47²) : zéro pour un brin manquant ou
    // pour une place inutilisée.
    fn fill(column: &mut [Gf2209], slots: &[Option<Box<[Gf47]>>], column_index: usize) {
        for (value, strand) in column.iter_mut().zip(slots) {
            *value = match strand {
                Some(strand) => pair(strand, column_index),
                None => Gf2209::zero(),
            };
        }
    }
    let mut column = vec![Gf2209::zero(); length];

    // Un bloc complet dont chaque colonne vérifie la parité est sain.
    if missing.is_empty() {
        let healthy = (0..payload / 2).all(|column_index| {
            fill(&mut column, slots, column_index);
            outer.check(&column)
        });
        if healthy {
            return Ok(0);
        }
    }

    // Un même jeu d'effacements pour toutes les colonnes : le résolveur les rétablit par
    // interpolation, et ne cède la place au décodage complet qu'en cas de valeur fausse.
    let solver = outer.solver(&missing).map_err(|error| {
        DnaError::InsufficientData(format!(
            "bloc {block} : code externe irrécupérable ({error})"
        ))
    })?;
    let mut touched = vec![false; count];
    for column_index in 0..payload / 2 {
        fill(&mut column, slots, column_index);
        if solver.restore(&mut column).is_err() {
            outer.correct(&mut column, &missing).map_err(|error| {
                DnaError::InsufficientData(format!(
                    "bloc {block} : code externe irrécupérable ({error})"
                ))
            })?;
        }
        // Toutes les valeurs corrigées sont relues : un brin présent peut avoir été corrigé.
        for (position, &value) in column.iter().enumerate() {
            if !layout.strand_exists(block, position) {
                continue;
            }
            let is_data = position < count;
            match slots[position].as_mut() {
                None => {
                    let mut strand = vec![Gf47::new(0); payload].into_boxed_slice();
                    strand[2 * column_index] = Gf47::new(value.a);
                    strand[2 * column_index + 1] = Gf47::new(value.b);
                    slots[position] = Some(strand);
                    if is_data {
                        touched[position] = true;
                    }
                }
                Some(strand) => {
                    if pair(strand, column_index) != value {
                        strand[2 * column_index] = Gf47::new(value.a);
                        strand[2 * column_index + 1] = Gf47::new(value.b);
                        if is_data {
                            touched[position] = true;
                        }
                    }
                }
            }
        }
    }
    Ok(touched.into_iter().filter(|&flag| flag).count())
}

/// Décode le corps de l'archive à partir des lectures (manifestes non compris).
pub fn decode(reads: &[Vec<u8>], body_len: usize, parity: usize) -> Result<(Vec<u8>, DecodeStats)> {
    let layout = GrassLayout::for_body(body_len, parity)?;
    let mut stats = DecodeStats::default();
    if layout.data_strands == 0 {
        return Ok((Vec::new(), stats));
    }
    let inner = Rs47Code::new(SYMBOLS, SYMBOLS - parity)?;
    let total = layout.total_strands;

    // 1. Code interne : chaque lecture de bonne longueur donne un index et une charge utile.
    let sized: Vec<&Vec<u8>> = reads
        .iter()
        .filter(|read| read.len() == STRAND_LEN)
        .collect();
    stats.reads_used = sized.len();
    let parsed: Vec<Parsed> = sized
        .par_iter()
        .filter_map(|read| parse_read(read, &inner, total).ok())
        .collect();
    stats.corrected = parsed.iter().filter(|(_, _, fixed)| *fixed > 0).count();
    stats.uncorrectable = sized.len() - parsed.len();

    // 2. Un brin par index ; à égalité ou à la minorité, le brin est écarté.
    let mut strands: HashMap<usize, IndexEntry> = HashMap::with_capacity(parsed.len());
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
    let mut slots: Vec<Option<Box<[Gf47]>>> = vec![None; total];
    for (index, entry) in strands {
        if entry.others >= entry.votes {
            stats.contested += 1;
        } else {
            stats.strands += 1;
            slots[index] = Some(entry.payload);
        }
    }

    // 3. Code externe, bloc par bloc.
    let outer = GrassLayout::outer_code();
    let repairs: Vec<Result<usize>> = slots
        .par_chunks_mut(BLOCK)
        .enumerate()
        .map(|(block, block_slots)| {
            repair_block(block, block_slots, &layout, &outer, layout.payload)
        })
        .collect();
    for repaired in repairs {
        let repaired = repaired?;
        stats.blocks_repaired += usize::from(repaired > 0);
        stats.strands_repaired += repaired;
    }

    // 4. Corps : les octets des brins de données, dans l'ordre.
    let mut body = Vec::with_capacity(layout.data_strands * layout.bytes_per_strand);
    for rank in 0..layout.data_strands {
        let index = (rank / BLOCK_DATA) * BLOCK + rank % BLOCK_DATA;
        let payload = slots[index].as_ref().ok_or_else(|| {
            DnaError::InsufficientData("brin de données manquant après le code externe".to_string())
        })?;
        body.extend_from_slice(&symbols_to_bytes(payload, layout.bytes_per_strand));
    }
    body.truncate(body_len);
    Ok((body, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::rng::SeededRng;
    use crate::sequence::{gc_content, gc_count, max_homopolymer};

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 23);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    fn shuffled(oligos: &[Vec<u8>], seed: u64) -> Vec<Vec<u8>> {
        let mut out = oligos.to_vec();
        let mut rng = SeededRng::new(seed, 24);
        for i in (1..out.len()).rev() {
            out.swap(i, rng.below(i as u64 + 1) as usize);
        }
        out
    }

    /// Remplace le symbole `position` d'un brin par le suivant : une erreur que rien ne signale.
    fn corrupt(strand: &mut [u8], position: usize) {
        let old = TABLES.1[trimer_code(&strand[SYMBOL_LEN * position..])];
        assert!(old >= 0, "le symbole doit être lisible");
        let next = (old as usize + 1) % 47;
        strand[SYMBOL_LEN * position..SYMBOL_LEN * position + SYMBOL_LEN]
            .copy_from_slice(&TABLES.0[next]);
    }

    /// Efface le symbole `position` : un trimère hors du jeu des 47.
    fn blank(strand: &mut [u8], position: usize) {
        strand[SYMBOL_LEN * position..SYMBOL_LEN * position + SYMBOL_LEN].copy_from_slice(b"AAA");
    }

    fn encode_body(body: &[u8], parity: usize) -> (Vec<Vec<u8>>, GrassLayout) {
        let (oligos, _) =
            encode(body, &GrassParams { parity }, &DnaConstraints::default()).unwrap();
        let layout = GrassLayout::for_body(body.len(), parity).unwrap();
        (oligos, layout)
    }

    /// Trois blocs pleins, parité par défaut : 1 863 brins de données, 276 de parité.
    fn three_blocks() -> (Vec<u8>, Vec<Vec<u8>>, GrassLayout) {
        let size = 3 * BLOCK_DATA * 13;
        let body = pseudo_random(size, 5);
        let (oligos, layout) = encode_body(&body, DEFAULT_PARITY);
        (body, oligos, layout)
    }

    #[test]
    fn the_47_trimers_never_leave_more_than_three_identical_bases() {
        let mut distinct = std::collections::HashSet::new();
        for (symbol, trimer) in TABLES.0.iter().enumerate() {
            assert!(
                distinct.insert(*trimer),
                "deux symboles ont le même trimère"
            );
            assert_eq!(TABLES.1[trimer_code(trimer)], symbol as i8);
        }
        // Deux trimères quelconques, bout à bout : jamais quatre bases identiques.
        for first in &TABLES.0 {
            for second in &TABLES.0 {
                let mut joined = first.to_vec();
                joined.extend_from_slice(second);
                assert!(max_homopolymer(&joined) <= 3);
            }
        }
        // 36 trimères sans répétition, 11 terminés par un doublet ; 17 écartés : les 4
        // homopolymères, les 12 à doublet initial, et CAA.
        assert_eq!(TABLES.1.iter().filter(|&&symbol| symbol == -1).count(), 17);
        assert!(TABLES.0.iter().any(|trimer| trimer == b"ACC"));
        assert!(TABLES.0.iter().any(|trimer| trimer == b"TGG"));
        for dropped in ["AAA", "CCA", "CAA", "GGT"] {
            assert_eq!(TABLES.1[trimer_code(dropped.as_bytes())], -1, "{dropped}");
        }
        // Soixante-onze bases G ou C sur 141 : la moitié, à une base près.
        let gc: usize = TABLES.0.iter().map(|trimer| gc_count(trimer)).sum();
        assert_eq!(gc, 71);
    }

    #[test]
    fn layout_follows_the_format() {
        // Parité 14 : 20 symboles de charge utile, 13 octets par brin.
        let empty = GrassLayout::for_body(0, 14).unwrap();
        assert_eq!(
            (
                empty.payload,
                empty.bytes_per_strand,
                empty.data_strands,
                empty.blocks
            ),
            (20, 13, 0, 0)
        );
        // Un seul brin de données : son bloc compte 93 brins.
        let one = GrassLayout::for_body(1, 14).unwrap();
        assert_eq!(
            (one.data_strands, one.blocks, one.total_strands),
            (1, 1, BLOCK)
        );
        assert_eq!(one.data_in_block(0), 1);
        assert!(one.strand_exists(0, 0) && one.strand_exists(0, BLOCK_DATA));
        assert!(!one.strand_exists(0, 1) && !one.strand_exists(0, BLOCK_DATA - 1));
        let full = GrassLayout::for_body(BLOCK_DATA * 13, 14).unwrap();
        assert_eq!(
            (full.data_strands, full.blocks, full.total_strands),
            (BLOCK_DATA, 1, BLOCK)
        );
        assert!(full.strand_exists(0, BLOCK_DATA - 1) && full.strand_exists(0, BLOCK - 1));
        let two = GrassLayout::for_body(BLOCK_DATA * 13 + 1, 14).unwrap();
        assert_eq!(
            (two.data_strands, two.blocks, two.total_strands),
            (BLOCK_DATA + 1, 2, 2 * BLOCK)
        );
        assert_eq!(two.data_in_block(1), 1);
        // Autant d'octets que le permettent les symboles, pas un de plus.
        for parity in [2, 6, 10, 14, 20, 30] {
            let layout = GrassLayout::for_body(1, parity).unwrap();
            assert_eq!(layout.payload + parity, 34);
            assert!(fits(layout.bytes_per_strand, layout.payload));
            assert!(!fits(layout.bytes_per_strand + 1, layout.payload));
        }
        assert_eq!(GrassLayout::for_body(1, 6).unwrap().bytes_per_strand, 19);
        assert_eq!(GrassLayout::for_body(1, 30).unwrap().bytes_per_strand, 2);
        assert_eq!(GrassLayout::for_body(1, 4).unwrap().bytes_per_strand, 20);
        assert_eq!(GrassLayout::for_body(1, 2).unwrap().bytes_per_strand, 22);
        // La parité est paire : les symboles de charge utile vont par paires.
        for bad in [0, 1, 3, 15, 21, 32, usize::MAX] {
            assert!(GrassLayout::for_body(1, bad).is_err(), "{bad}");
        }
        assert!(GrassParams { parity: 15 }.validate().is_err());
        assert!(GrassParams { parity: 16 }.validate().is_ok());
        assert!(GrassLayout::for_body(usize::MAX, 14).is_err());
        // 6 843 blocs de 713 brins tiennent dans l'index de 47^4 ; un de plus, non.
        assert!(GrassLayout::for_body(6_843 * BLOCK_DATA * 13, 14).is_ok());
        assert!(GrassLayout::for_body(6_844 * BLOCK_DATA * 13, 14).is_err());
    }

    #[test]
    fn bytes_and_symbols_round_trip_through_base_47() {
        for (bytes, symbols) in [(13usize, 20usize), (19, 28), (7, 11), (3, 5), (2, 4)] {
            let data = pseudo_random(bytes, bytes as u64);
            let encoded: Vec<Gf47> = bytes_to_symbols(&data, symbols)
                .into_iter()
                .map(Gf47::new)
                .collect();
            assert_eq!(encoded.len(), symbols);
            assert!(encoded.iter().all(|symbol| usize::from(symbol.0) < 47));
            assert_eq!(symbols_to_bytes(&encoded, bytes), data);
            // Tous les octets à 255 tiennent aussi.
            let full: Vec<Gf47> = bytes_to_symbols(&vec![0xFF; bytes], symbols)
                .into_iter()
                .map(Gf47::new)
                .collect();
            assert_eq!(symbols_to_bytes(&full, bytes), vec![0xFF; bytes]);
        }
        assert!(fits(0, 0));
        assert!(!fits(1, 1));
        assert!(fits(2, 4));
        assert!(!fits(3, 4));
    }

    #[test]
    fn roundtrip_and_every_strand_is_compliant() {
        let constraints = DnaConstraints::default();
        for parity in [6, DEFAULT_PARITY] {
            for size in [0usize, 1, 12, 13, 14, 5_000, 100_000] {
                let body = pseudo_random(size, size as u64 + parity as u64);
                let (oligos, stats) = encode(&body, &GrassParams { parity }, &constraints).unwrap();
                let layout = GrassLayout::for_body(size, parity).unwrap();
                assert_eq!(oligos.len(), stats.data_strands + stats.parity_strands);
                assert_eq!(stats.parity_strands, layout.blocks * BLOCK_PARITY);
                let mut gc = 0.0;
                for oligo in &oligos {
                    assert_eq!(oligo.len(), STRAND_LEN);
                    constraints
                        .check(oligo)
                        .unwrap_or_else(|error| panic!("parité {parity}, taille {size} : {error}"));
                    gc += gc_content(oligo);
                }
                // Le GC criblé est centré sur 50 %, au bruit d'échantillonnage près.
                if oligos.len() >= 200 {
                    assert!(
                        (gc / oligos.len() as f64 - 0.5).abs() < 0.02,
                        "parité {parity}, taille {size} : GC moyen {:.4} sur {} brins",
                        gc / oligos.len() as f64,
                        oligos.len()
                    );
                }
                // En moyenne moins de deux essais de nonce par brin.
                if !oligos.is_empty() {
                    assert!(stats.nonces_tried < oligos.len() as u64 * 64);
                }
                let (decoded, decode_stats) = decode(&oligos, size, parity).unwrap();
                assert_eq!(decoded, body, "parité {parity}, taille {size}");
                assert_eq!(decode_stats.blocks_repaired, 0);
                assert_eq!(decode_stats.uncorrectable, 0);
                assert_eq!(decode_stats.strands, oligos.len());
                assert_eq!(decode(&shuffled(&oligos, 3), size, parity).unwrap().0, body);
            }
        }
        // Sans contrainte de GC, la limite de trois bases tient toujours.
        let body = pseudo_random(20_000, 7);
        let (oligos, _) = encode(
            &body,
            &GrassParams::default(),
            &DnaConstraints::new(0.0, 1.0, 3),
        )
        .unwrap();
        assert!(oligos.iter().all(|oligo| max_homopolymer(oligo) <= 3));
        assert_eq!(decode(&oligos, 20_000, DEFAULT_PARITY).unwrap().0, body);
    }

    #[test]
    fn the_inner_code_corrects_half_the_parity_in_symbols() {
        // Parité 6 : trois symboles erronés par lecture, index compris.
        let size = 3_000;
        let parity = 6;
        let body = pseudo_random(size, 21);
        let (oligos, _) = encode_body(&body, parity);
        let mut damaged = oligos.clone();
        for (rank, strand) in damaged.iter_mut().enumerate() {
            for position in [0, 10 + rank % 20, 38] {
                corrupt(strand, position);
            }
        }
        let (decoded, stats) = decode(&damaged, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, oligos.len());
        assert_eq!(stats.uncorrectable, 0);
        assert_eq!(stats.blocks_repaired, 0);

        // Quatre erreurs : au-delà. Les lectures perdues sont rétablies par le code externe ;
        // 93 d'entre elles dans un même bloc, plus rien.
        let mut damaged = oligos.clone();
        for strand in &mut damaged[..4] {
            for position in [0, 10, 20, 38] {
                corrupt(strand, position);
            }
        }
        let (decoded, stats) = decode(&damaged, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 4);
        assert_eq!(stats.blocks_repaired, 1);
        assert_eq!(stats.strands_repaired, 4);
        for strand in damaged.iter_mut().take(93) {
            for position in [0, 10, 20, 38] {
                corrupt(strand, position);
            }
        }
        let err = decode(&damaged, size, parity).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");
    }

    #[test]
    fn a_trimer_outside_the_alphabet_is_an_erasure() {
        // Parité 14 : quatorze effacements, ou sept effacements et trois erreurs (7 + 2 × 3).
        let size = 3_000;
        let body = pseudo_random(size, 31);
        let (oligos, _) = encode_body(&body, DEFAULT_PARITY);
        let mut damaged = oligos.clone();
        for strand in &mut damaged {
            for position in [0, 2, 4, 8, 15, 21, 33] {
                blank(strand, position);
            }
            for position in [10, 20, 38] {
                corrupt(strand, position);
            }
        }
        let (decoded, stats) = decode(&damaged, size, DEFAULT_PARITY).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, oligos.len());
        assert_eq!(stats.uncorrectable, 0);

        // Quinze effacements : la lecture est perdue, puis rétablie par le code externe.
        let mut damaged = oligos.clone();
        for position in 0..15 {
            blank(&mut damaged[9], position);
        }
        let (decoded, stats) = decode(&damaged, size, DEFAULT_PARITY).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 1);
        assert_eq!(stats.strands_repaired, 1);
    }

    #[test]
    fn an_error_in_the_nonce_is_repaired_by_the_other_nonces() {
        let size = 2_000;
        let body = pseudo_random(size, 41);
        let (oligos, _) = encode_body(&body, DEFAULT_PARITY);
        // Symbole de nonce faux, mais valide : le masque est faux.
        let mut damaged = oligos.clone();
        for strand in &mut damaged {
            corrupt(strand, INDEX_SYMBOLS);
        }
        let (decoded, stats) = decode(&damaged, size, DEFAULT_PARITY).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 0);
        assert_eq!(stats.blocks_repaired, 0);
        // Trimère de nonce illisible.
        let mut damaged = oligos.clone();
        for strand in &mut damaged {
            blank(strand, INDEX_SYMBOLS);
        }
        let (decoded, stats) = decode(&damaged, size, DEFAULT_PARITY).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 0);
    }

    #[test]
    fn missing_strands_are_restored_by_the_outer_code() {
        let (body, oligos, layout) = three_blocks();
        assert_eq!(oligos.len(), 3 * BLOCK);
        let size = body.len();
        let parity = layout.parity;

        // Quatre-vingt-dix brins de données du premier bloc manquants.
        let kept: Vec<Vec<u8>> = oligos.iter().skip(90).cloned().collect();
        let (decoded, stats) = decode(&kept, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.blocks_repaired, 1);
        assert_eq!(stats.strands_repaired, 90);

        // Soixante-dix données et vingt-deux parités du premier bloc manquantes : 92, la limite.
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                !(0..70).contains(index) && !(BLOCK_DATA + 50..BLOCK_DATA + 72).contains(index)
            })
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let (decoded, stats) = decode(&kept, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.strands_repaired, 70);

        // Quatre-vingt-treize données manquantes : le bloc est perdu, l'échec est annoncé.
        let kept: Vec<Vec<u8>> = oligos.iter().skip(93).cloned().collect();
        let err = decode(&kept, size, parity).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");

        // Seuls des brins de parité manquent, dans les trois blocs : rien à réparer côté données.
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| index % BLOCK < BLOCK_DATA)
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let (decoded, stats) = decode(&kept, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.blocks_repaired, 0);

        // Un bloc entier manque au milieu : il est perdu, l'échec est annoncé.
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| !(BLOCK..2 * BLOCK).contains(index))
            .map(|(_, oligo)| oligo.clone())
            .collect();
        assert!(decode(&kept, size, parity).is_err());

        // Le dernier bloc incomplet : ses places vides comptent pour zéro.
        let size = (2 * BLOCK_DATA + 3) * 13;
        let body = pseudo_random(size, 53);
        let (oligos, _) = encode_body(&body, DEFAULT_PARITY);
        assert_eq!(oligos.len(), 2 * BLOCK_DATA + 3 + 3 * BLOCK_PARITY);
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| !(2 * BLOCK..2 * BLOCK + 3).contains(index))
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let (decoded, stats) = decode(&kept, size, DEFAULT_PARITY).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.strands_repaired, 3);
    }

    /// Une lecture « corrigée » à tort : même index, parité interne valide, autre charge utile.
    fn forged(strand: &[u8], total: usize, constraints: &DnaConstraints, parity: usize) -> Vec<u8> {
        let inner = Rs47Code::new(SYMBOLS, SYMBOLS - parity).unwrap();
        let (index, mut payload, _) = parse_read(strand, &inner, total).unwrap();
        payload[0] = Gf47::new(((usize::from(payload[0].0) + 1) % 47) as u8);
        build_strand(index, &payload, &inner, constraints, &mut 0).unwrap()
    }

    #[test]
    fn a_present_but_false_strand_is_corrected_by_the_outer_code() {
        let (body, oligos, layout) = three_blocks();
        let total = layout.total_strands;
        let constraints = DnaConstraints::default();
        let (size, parity) = (body.len(), layout.parity);

        // Deux brins faux dans le premier bloc, un dans le deuxième, un brin de parité du
        // troisième.
        let mut reads = oligos.clone();
        for rank in [0, 7, BLOCK + 5, 2 * BLOCK + BLOCK_DATA + 1] {
            reads[rank] = forged(&oligos[rank], total, &constraints, parity);
        }
        let (decoded, stats) = decode(&reads, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.corrected, 0, "leur parité interne est valide");
        assert_eq!(stats.blocks_repaired, 2);
        assert_eq!(
            stats.strands_repaired, 3,
            "les brins de parité ne comptent pas"
        );

        // Quarante-six brins faux dans un bloc : 92 places, la limite exacte, et ça passe.
        let mut reads = oligos.clone();
        for rank in 0..46 {
            reads[rank] = forged(&oligos[rank], total, &constraints, parity);
        }
        let (decoded, stats) = decode(&reads, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.blocks_repaired, 1);
        assert_eq!(stats.strands_repaired, 46);

        // Quarante-sept : le bloc est perdu.
        reads[100] = forged(&oligos[100], total, &constraints, parity);
        let err = decode(&reads, size, parity).unwrap_err();
        assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");

        // Quatre-vingt-dix manquants et un faux : 90 + 2 = 92, la limite, et ça passe.
        let mut reads: Vec<Vec<u8>> = oligos[90..].to_vec();
        reads[10] = forged(&oligos[100], total, &constraints, parity);
        let (decoded, stats) = decode(&reads, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.strands_repaired, 91);

        // Quatre-vingt-onze manquants et un faux : 93, plus rien.
        let mut reads: Vec<Vec<u8>> = oligos[91..].to_vec();
        reads[9] = forged(&oligos[100], total, &constraints, parity);
        assert!(decode(&reads, size, parity).is_err());
    }

    #[test]
    fn conflicting_reads_for_one_index_are_settled_by_majority() {
        let (body, oligos, layout) = three_blocks();
        let total = layout.total_strands;
        let (size, parity) = (body.len(), layout.parity);
        let constraints = DnaConstraints::default();
        let false_read = forged(&oligos[5], total, &constraints, parity);

        // Égalité : le brin est écarté, puis rétabli par le code externe.
        let mut reads = oligos.clone();
        reads.push(false_read.clone());
        let (decoded, stats) = decode(&reads, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 1);
        assert_eq!(stats.strands_repaired, 1);

        // Majorité : la bonne charge utile l'emporte.
        let mut reads = oligos.clone();
        reads.push(oligos[5].clone());
        reads.push(false_read);
        let (decoded, stats) = decode(&reads, size, parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 0);
        assert_eq!(stats.blocks_repaired, 0);
    }

    #[test]
    fn duplicate_reads_and_wrong_lengths_are_harmless() {
        let (body, oligos, layout) = three_blocks();
        let mut reads: Vec<Vec<u8>> = oligos.iter().flat_map(|o| [o.clone(), o.clone()]).collect();
        reads.push(vec![b'A'; STRAND_LEN - 1]);
        reads.push(vec![b'A'; STRAND_LEN + 4]);
        reads.push(Vec::new());
        let (decoded, stats) = decode(&reads, body.len(), layout.parity).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.uncorrectable, 0);
        assert_eq!(stats.reads_used, reads.len() - 3);
    }

    #[test]
    fn constraints_the_nonces_cannot_satisfy_are_refused_up_front() {
        // Une fenêtre de GC de 49 % à 51 % laisse, pour chaque brin, six chances sur dix mille
        // qu'aucun des 47 nonces ne convienne : refusé d'emblée.
        let err = encode(
            &[0u8; 500],
            &GrassParams::default(),
            &DnaConstraints::new(0.49, 0.51, 3),
        )
        .expect_err("contraintes trop strictes");
        assert!(matches!(err, DnaError::Encoding(_)), "{err}");
        assert!(err.to_string().contains("117 nt"), "{err}");
        let loose = DnaConstraints::new(0.40, 0.60, 3);
        assert!(encode(&[0u8; 500], &GrassParams::default(), &loose).is_ok());
        assert!(GrassParams { parity: 31 }.validate().is_err());
        assert!(GrassParams::default().validate().is_ok());
    }
}
