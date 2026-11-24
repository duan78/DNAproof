//! Schéma « ldpc » : code convolutif par brin, LDPC par bloc, décodage à décisions souples.
//!
//! Deux codes se succèdent, sur l'image binaire du canal — deux bits par base :
//!
//! - **Interne**, brin par brin : le code convolutif (171, 133)₈, longueur de contrainte 7,
//!   rendement ½. Il s'accommode des erreurs dispersées comme des salves, et sa sortie est
//!   souple : un rapport de vraisemblance par bit.
//! - **Externe**, bloc par bloc : un LDPC à 819 brins de données, reconstruit d'une graine,
//!   décodé par propagation de croyances. Il répare les brins manquants — autant d'effacements —
//!   et les bits faux qui restent.
//!
//! Entre les deux, **l'entrelacement** : dans le mot de code d'un bloc, les bits de même rang
//! passent consécutivement. Un brin manquant, salve de cent douze effacements, devient cent
//! douze effacements isolés.
//!
//! Un brin fait 154 nt : un bit d'information par base, puisque le code est de rendement ½ et
//! que chaque base porte deux bits de code.
//!
//! | sel, 20 bits | index, 24 bits | charge utile, 13 octets | queue, 6 bits |
//! |---|---|---|---|
//! | en clair, règle le masquage | dispersé, en clair | masquée par un flux dérivé de l'index et du sel | ramène l'encodeur à zéro |
//!
//! À l'encodage, l'encodeur fait varier le sel jusqu'à obtenir un brin qui respecte les
//! contraintes biochimiques : comme le sel règle aussi le masquage, tout le brin en dépend.
//! Il vient en tête, là où l'encodeur part de zéro : sans lui, les premières bases ne
//! dépendraient de rien.
//!
//! Au décodage, chaque lecture passe sur le treillis (algorithme log-MAP), qui donne son index
//! et le rapport a posteriori de chacun de ses bits utiles. Les lectures d'un même brin
//! s'additionnent ; un brin sans lecture garde des rapports nuls, ce sont des effacements. Le
//! LDPC tranche. Les rapports d'une lecture sont bornés : une lecture mal décodée est
//! affirmée, et ne doit pas l'emporter sur les lectures justes du même brin. Les rapports sont
//! pris à une échelle fixe : le min-sum n'en dépend pas, et les décisions du treillis non plus.

use crate::codec::rng::{scramble_u24, unscramble_u24, SeededRng};
use crate::constraints::DnaConstraints;
use crate::convolutional::{bcjr, encode as convolve, viterbi, TAIL};
use crate::error::{DnaError, Result};
use crate::ldpc::LdpcCode;
use crate::sequence::BASES;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Longueur d'un brin, en nucléotides.
pub const STRAND_LEN: usize = 154;
/// Bits de code d'un brin : deux par base.
pub const CODED_BITS: usize = 2 * STRAND_LEN;
/// Bits d'information d'un brin : le rendement ½ donne un bit par base, la queue en prend six.
pub const INFO_BITS: usize = STRAND_LEN - TAIL;
/// Bits d'index : autant que de brins adressables, seize millions.
pub const INDEX_BITS: usize = 24;
/// Bits de sel, pour le criblage : un million d'essais par brin.
pub const SALT_BITS: usize = 20;
/// Bits de charge utile d'un brin.
pub const PAYLOAD_BITS: usize = INFO_BITS - INDEX_BITS - SALT_BITS;
/// Octets de charge utile d'un brin.
pub const PAYLOAD_LEN: usize = PAYLOAD_BITS / 8;
/// Brins de données par bloc.
pub const BLOCK_DATA: usize = 819;
/// Parité par défaut : brins de parité pour cent brins de données.
pub const DEFAULT_PARITY: usize = 20;
/// Parité minimale, pour cent.
pub const MIN_PARITY: usize = 5;
/// Parité maximale, pour cent.
pub const MAX_PARITY: usize = 100;
/// Nombre total de brins que l'index permet d'adresser.
pub const MAX_STRANDS: u32 = 1 << INDEX_BITS;
/// Itérations de propagation de croyances au plus, par bloc.
pub const MAX_ITERATIONS: usize = 60;

/// Échelle fixe des rapports de vraisemblance : chaque base lue est crue avec ce poids.
const CONFIDENCE: f32 = 4.0;
const WHITEN_STREAM: u64 = 0x0000_0000_0000_0014;
const INDEX_MASK_STREAM: u64 = 0x0000_0000_0000_0015;
/// Graine du LDPC : la matrice de contrôle est la même pour tous les blocs de même taille.
const CODE_SEED: u64 = 0x0000_0000_4C44_5043;

/// Paramètres du schéma ldpc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LdpcParams {
    /// Brins de parité pour cent brins de données.
    pub parity: usize,
}

impl Default for LdpcParams {
    fn default() -> Self {
        Self {
            parity: DEFAULT_PARITY,
        }
    }
}

impl LdpcParams {
    /// Vérifie la parité.
    pub fn validate(&self) -> Result<()> {
        if (MIN_PARITY..=MAX_PARITY).contains(&self.parity) {
            Ok(())
        } else {
            Err(DnaError::InvalidConfig(format!(
                "parité LDPC invalide : {} (attendu : {MIN_PARITY} à {MAX_PARITY} brins de \
                 parité pour cent brins de données)",
                self.parity
            )))
        }
    }
}

/// Brins de parité de `data` brins de données, à `parity` pour cent.
pub fn parity_of(data: usize, parity: usize) -> usize {
    (data * parity).div_ceil(100)
}

/// Répartition d'un corps en brins et en blocs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LdpcLayout {
    pub data_strands: u32,
    pub blocks: u32,
    pub parity_strands: u32,
    pub total_strands: u32,
    pub parity: usize,
}

fn too_large() -> DnaError {
    DnaError::Encoding(format!(
        "fichier trop volumineux pour le schéma ldpc (limite : {MAX_STRANDS} brins d'index)"
    ))
}

impl LdpcLayout {
    /// Répartition pour un corps de `body_len` octets, à cette parité, si le format peut le
    /// contenir.
    pub fn for_body(body_len: usize, parity: usize) -> Result<Self> {
        LdpcParams { parity }.validate()?;
        let data_strands =
            u32::try_from(body_len.div_ceil(PAYLOAD_LEN)).map_err(|_| too_large())?;
        let blocks = data_strands.div_ceil(BLOCK_DATA as u32);
        let parity_strands = (0..blocks as usize)
            .map(|block| parity_of(Self::data_in(data_strands, block), parity))
            .try_fold(0u32, |sum, value| {
                sum.checked_add(value as u32).ok_or_else(too_large)
            })?;
        let total_strands = data_strands
            .checked_add(parity_strands)
            .filter(|&total| total < MAX_STRANDS)
            .ok_or_else(too_large)?;
        Ok(Self {
            data_strands,
            blocks,
            parity_strands,
            total_strands,
            parity,
        })
    }

    /// Brins de données du bloc `block`, parmi `data` en tout.
    fn data_in(data: u32, block: usize) -> usize {
        (data as usize)
            .saturating_sub(block.saturating_mul(BLOCK_DATA))
            .min(BLOCK_DATA)
    }

    /// Brins de données du bloc `block`.
    pub fn data_in_block(&self, block: usize) -> usize {
        Self::data_in(self.data_strands, block)
    }

    /// Brins de parité du bloc `block`.
    pub fn parity_in_block(&self, block: usize) -> usize {
        parity_of(self.data_in_block(block), self.parity)
    }

    /// Taille des blocs complets : brins de données et de parité.
    fn full_block(&self) -> usize {
        BLOCK_DATA + parity_of(BLOCK_DATA, self.parity)
    }

    /// Premier brin du bloc `block`.
    pub fn first_strand(&self, block: usize) -> usize {
        block * self.full_block()
    }

    /// Nombre de brins du bloc `block`.
    pub fn strands_in_block(&self, block: usize) -> usize {
        self.data_in_block(block) + self.parity_in_block(block)
    }
}

/// Bilan d'un encodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub data_strands: u32,
    pub parity_strands: u32,
    /// Sels essayés pour satisfaire les contraintes.
    pub salts_tried: u64,
}

/// Bilan d'un décodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Lectures de la longueur attendue.
    pub reads_used: usize,
    /// Lectures que le treillis a corrigées.
    pub corrected: usize,
    /// Lectures dont l'index est hors de l'archive.
    pub uncorrectable: usize,
    /// Brins distincts lus, avant le LDPC.
    pub strands: usize,
    /// Blocs où le LDPC a changé ou restitué un brin.
    pub blocks_repaired: usize,
    /// Brins changés ou restitués par le LDPC.
    pub strands_repaired: usize,
    /// Plus grand nombre d'itérations faites sur un bloc.
    pub iterations: usize,
}

/// Masque du sel.
const SALT_MASK: u32 = (1 << SALT_BITS) - 1;

/// Le sel tel qu'il est transmis : une bijection de 20 bits, pour que les petits nombres ne
/// s'écrivent pas en longues suites de zéros, donc de A, en tête du brin.
fn wire_salt(salt: u32) -> u32 {
    let mut x = (u64::from(salt) * 0x6_E5D1 + 0x2_7F3A) as u32 & SALT_MASK;
    x ^= x >> 10;
    x
}

/// L'inverse de [`wire_salt`].
fn unwire_salt(wire: u32) -> u32 {
    let mut x = wire & SALT_MASK;
    x ^= x >> 10;
    let inverse = crate::codec::rng::inverse_odd(0x6_E5D1) as u32 & SALT_MASK;
    (x.wrapping_sub(0x2_7F3A) & SALT_MASK).wrapping_mul(inverse) & SALT_MASK
}

/// De quoi masquer l'index dispersé, 24 bits dérivés du seul sel : le décodeur lit le sel
/// avant de connaître l'index.
fn index_mask(salt: u32) -> u32 {
    let mut key = [0u8; INDEX_BITS / 8];
    SeededRng::new(u64::from(salt), INDEX_MASK_STREAM).fill_bytes(&mut key);
    u32::from(key[0]) | u32::from(key[1]) << 8 | u32::from(key[2]) << 16
}

/// Flux de masquage de la charge utile d'un brin : il dépend de l'index et du sel.
fn keystream(index: u32, salt: u32) -> [u8; PAYLOAD_LEN] {
    let mut key = [0u8; PAYLOAD_LEN];
    let seed = u64::from(index) << SALT_BITS | u64::from(salt);
    SeededRng::new(seed, WHITEN_STREAM).fill_bytes(&mut key);
    key
}

/// Bits d'information du brin, poids fort en tête : sel dispersé, index dispersé et masqué,
/// charge utile masquée.
fn information(index: u32, salt: u32, payload: &[u8], key: &[u8; PAYLOAD_LEN]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(INFO_BITS);
    for (value, width) in [
        (wire_salt(salt), SALT_BITS),
        (scramble_u24(index) ^ index_mask(salt), INDEX_BITS),
    ] {
        for shift in (0..width as u32).rev() {
            bits.push(((value >> shift) & 1) as u8);
        }
    }
    for (value, mask) in payload.iter().zip(key) {
        for shift in (0..8).rev() {
            bits.push(((value ^ mask) >> shift) & 1);
        }
    }
    bits
}

/// Rang du bit `bit` du brin `strand` dans le mot de code d'un bloc de `strands` brins.
///
/// C'est l'entrelacement : les bits d'un même rang passent consécutivement, et ceux d'un
/// même brin se trouvent à `strands` positions l'un de l'autre. Un brin manquant est une
/// salve de cent douze effacements ; l'entrelacement la disperse en effacements isolés, que
/// la propagation éteint en peu d'itérations. Sans lui, une salve qui tombe sur la chaîne de
/// parité ne se résout que bit à bit, le long de la chaîne.
fn position(strands: usize, bit: usize, strand: usize) -> usize {
    bit * strands + strand
}

/// Bit de rang `bit` d'un octet, poids fort en tête.
fn bit_of(byte: u8, bit: usize) -> u8 {
    (byte >> (7 - bit % 8)) & 1
}

/// Octet de huit bits, poids fort en tête.
fn byte_of(bits: impl Iterator<Item = u8>) -> u8 {
    bits.fold(0u8, |value, bit| value << 1 | bit)
}

/// Nombre de sels à essayer pour qu'un brin conforme en sorte, sauf une chance sur 10¹⁷ ;
/// `None` si l'espace des sels ne suffit pas.
///
/// Les sorties du code convolutif se répètent plus souvent que des bases tirées au sort : aux
/// contraintes par défaut, 3,3 % des brins y satisfont, contre 13 % de séquences au hasard
/// (mesuré). La probabilité d'une séquence au hasard est donc prise pour son quart.
fn salt_limit(constraints: &DnaConstraints) -> Result<u32> {
    const STRUCTURE: f64 = 4.0;
    let probability = constraints.pass_probability(STRAND_LEN) / STRUCTURE;
    let needed = (40.0 / probability).ceil();
    if probability > 0.0 && needed.is_finite() && needed <= f64::from(1u32 << SALT_BITS) {
        Ok(needed.max(1.0) as u32)
    } else {
        Err(DnaError::Encoding(
            constraints.too_strict_message(STRAND_LEN),
        ))
    }
}

/// Le brin d'index et de charge utile donnés. Le sel varie jusqu'à ce que le brin respecte les
/// contraintes.
fn build_strand(
    index: u32,
    payload: &[u8],
    constraints: &DnaConstraints,
    salts_tried: &mut u64,
    salt_limit: u32,
) -> Result<Vec<u8>> {
    let mut dna = Vec::with_capacity(STRAND_LEN);
    for salt in 0..salt_limit {
        *salts_tried += 1;
        let coded = convolve(&information(index, salt, payload, &keystream(index, salt)));
        dna.clear();
        for pair in coded.as_chunks::<2>().0 {
            dna.push(BASES[usize::from(pair[0] << 1 | pair[1])]);
        }
        if constraints.satisfied(&dna) {
            return Ok(dna);
        }
    }
    Err(DnaError::Encoding(format!(
        "aucun sel ne donne un brin conforme pour l'index {index} : contraintes trop strictes"
    )))
}

/// Une lecture passée sur le treillis.
struct Parsed {
    /// Index de son brin.
    index: u32,
    /// Rapports a posteriori de la charge utile, démasqués.
    llrs: Box<[f32]>,
    /// Bits de code que le treillis a corrigés.
    fixed: usize,
}

/// Ce que le canal dit de chaque bit de code d'une lecture : chaque base lue est crue avec le
/// même poids, pour chacun de ses deux bits.
fn received(read: &[u8]) -> Vec<f32> {
    let mut llrs = Vec::with_capacity(CODED_BITS);
    for &base in read {
        let value = match base {
            b'A' => 0,
            b'C' => 1,
            b'G' => 2,
            _ => 3,
        };
        llrs.push(if value & 2 == 0 {
            CONFIDENCE
        } else {
            -CONFIDENCE
        });
        llrs.push(if value & 1 == 0 {
            CONFIDENCE
        } else {
            -CONFIDENCE
        });
    }
    llrs
}

/// Passe une lecture sur le treillis.
fn parse_read(read: &[u8]) -> Parsed {
    let llrs = received(read);
    let hard: Vec<u8> = llrs.iter().map(|&llr| u8::from(llr < 0.0)).collect();
    // Viterbi d'abord, plus de dix fois plus vite. Si son chemin redonne la lecture à l'identique,
    // la lecture est un mot de code : ses rapports a posteriori dépassent tous la borne. Bornés,
    // ils valent donc le poids du canal, du signe de chaque bit — et le log-MAP n'a rien à ajouter.
    // Sinon, le log-MAP dit quel poids accorder à chaque bit d'une lecture fautive.
    let (bits, posterior, fixed) = match viterbi(&llrs) {
        Some(bits) if convolve(&bits) == hard => (bits, None, 0),
        _ => {
            let soft = bcjr(&llrs, &vec![0.0; INFO_BITS]).expect("longueur d'un brin");
            // Bits corrigés : la lecture, face au mot de code redécidé.
            let fixed = convolve(&soft.bits)
                .iter()
                .zip(&hard)
                .filter(|(&bit, &received)| bit != received)
                .count();
            (soft.bits, Some(soft.llrs), fixed)
        }
    };
    let salt = unwire_salt(
        bits[..SALT_BITS]
            .iter()
            .fold(0u32, |salt, &bit| salt << 1 | u32::from(bit)),
    );
    let wire = bits[SALT_BITS..SALT_BITS + INDEX_BITS]
        .iter()
        .fold(0u32, |wire, &bit| wire << 1 | u32::from(bit))
        ^ index_mask(salt);
    let index = unscramble_u24(wire);
    let key = keystream(index, salt);
    let payload: Vec<f32> = (0..PAYLOAD_BITS)
        .map(|position| {
            let mask = (key[position / 8] >> (7 - position % 8)) & 1;
            let at = SALT_BITS + INDEX_BITS + position;
            match &posterior {
                None => {
                    let value = bits[at] ^ mask;
                    if value == 0 {
                        CONFIDENCE
                    } else {
                        -CONFIDENCE
                    }
                }
                // Borne : une lecture mal décodée est affirmée, et sans borne elle l'emporterait sur
                // les lectures justes du même brin. Chaque lecture pèse au plus ce que le canal en
                // soutient.
                Some(posterior) => {
                    let llr = posterior[at].clamp(-CONFIDENCE, CONFIDENCE);
                    if mask == 1 {
                        -llr
                    } else {
                        llr
                    }
                }
            }
        })
        .collect();
    Parsed {
        index,
        llrs: payload.into_boxed_slice(),
        fixed,
    }
}
/// Le LDPC d'un bloc : `data` brins de données, `parity` brins de parité.
fn block_code(data: usize, parity: usize) -> LdpcCode {
    LdpcCode::new(data * PAYLOAD_BITS, parity * PAYLOAD_BITS, CODE_SEED)
}

/// Encode le corps d'une archive en brins (manifestes non compris).
pub fn encode(
    body: &[u8],
    params: &LdpcParams,
    constraints: &DnaConstraints,
) -> Result<(Vec<Vec<u8>>, EncodeStats)> {
    let layout = LdpcLayout::for_body(body.len(), params.parity)?;
    let data = layout.data_strands as usize;
    let mut padded = body.to_vec();
    padded.resize(data * PAYLOAD_LEN, 0);
    let mut payloads: Vec<[u8; PAYLOAD_LEN]> =
        vec![[0; PAYLOAD_LEN]; layout.total_strands as usize];
    // Chaque brin de données prend rang dans son bloc, après la parité du bloc précédent.
    for (number, chunk) in padded.chunks(PAYLOAD_LEN).enumerate() {
        let (block, position) = (number / BLOCK_DATA, number % BLOCK_DATA);
        payloads[layout.first_strand(block) + position].copy_from_slice(chunk);
    }

    // Code externe : la parité de chaque bloc, brin par brin.
    let mut full_code: Option<LdpcCode> = None;
    for block in 0..layout.blocks as usize {
        let (data_in, parity_in, first) = (
            layout.data_in_block(block),
            layout.parity_in_block(block),
            layout.first_strand(block),
        );
        // Les blocs complets partagent la même matrice ; le dernier, plus court, a la sienne.
        let short = (data_in != BLOCK_DATA).then(|| block_code(data_in, parity_in));
        let code = match short.as_ref() {
            Some(short) => short,
            None => full_code.get_or_insert_with(|| block_code(data_in, parity_in)),
        };
        let mut info = vec![0u8; data_in * PAYLOAD_BITS];
        for strand in 0..data_in {
            for bit in 0..PAYLOAD_BITS {
                info[position(data_in, bit, strand)] =
                    bit_of(payloads[first + strand][bit / 8], bit);
            }
        }
        let codeword = code.encode(&info)?;
        let parity_bits = &codeword[data_in * PAYLOAD_BITS..];
        for strand in 0..parity_in {
            let mut payload = [0u8; PAYLOAD_LEN];
            for (byte, slot) in payload.iter_mut().enumerate() {
                let bits =
                    (0..8).map(|bit| parity_bits[position(parity_in, byte * 8 + bit, strand)]);
                *slot = byte_of(bits);
            }
            payloads[first + data_in + strand] = payload;
        }
    }

    // Essais de sel au-delà desquels la recherche est vaine : refusée d'emblée si l'espace des
    // sels ne suffit pas.
    let salt_limit = salt_limit(constraints)?;
    let salts_tried = AtomicU64::new(0);
    let oligos: Vec<Vec<u8>> = payloads
        .par_iter()
        .enumerate()
        .map_init(
            || 0u64,
            |local, (index, payload)| {
                let outcome = build_strand(index as u32, payload, constraints, local, salt_limit);
                salts_tried.fetch_add(*local, Ordering::Relaxed);
                *local = 0;
                outcome
            },
        )
        .collect::<Result<Vec<_>>>()?;

    let stats = EncodeStats {
        data_strands: layout.data_strands,
        parity_strands: layout.parity_strands,
        salts_tried: salts_tried.load(Ordering::Relaxed),
    };
    Ok((oligos, stats))
}

/// Ce qu'un bloc rend : son corps, ses brins réparés, ses itérations.
struct BlockRepair {
    body: Vec<u8>,
    repaired: usize,
    iterations: usize,
}

/// Décode un bloc : les rapports de ses brins entrent dans le LDPC, qui rend ses données.
fn repair_block(
    layout: &LdpcLayout,
    block: usize,
    soft: &[Option<Box<[f32]>>],
    full_code: Option<&LdpcCode>,
    last_code: Option<&LdpcCode>,
) -> Result<BlockRepair> {
    let (data_in, parity_in, first) = (
        layout.data_in_block(block),
        layout.parity_in_block(block),
        layout.first_strand(block),
    );
    let strands = data_in + parity_in;
    // Rang, dans le mot de code, du bit `bit` du brin `strand` : les données d'abord, entrelacées
    // entre brins de données, puis la parité, entrelacée entre brins de parité.
    let at = |strand: usize, bit: usize| {
        if strand < data_in {
            position(data_in, bit, strand)
        } else {
            data_in * PAYLOAD_BITS + position(parity_in, bit, strand - data_in)
        }
    };
    let code = if data_in == BLOCK_DATA {
        full_code
    } else {
        last_code
    }
    .expect("un code par taille de bloc");
    let mut channel = vec![0f32; strands * PAYLOAD_BITS];
    let (mut read, mut missing) = (0, 0);
    for (strand, sum) in soft[first..first + strands].iter().enumerate() {
        if let Some(sum) = sum {
            read += 1;
            for (bit, &llr) in sum.iter().enumerate() {
                channel[at(strand, bit)] = llr;
            }
        } else {
            missing += 1;
        }
    }
    // Un bloc presque vide rendrait un mot de code au hasard ; un bloc vide, le mot nul, qui
    // satisfait tous ses contrôles. La parité ne comblera jamais plus de brins qu'elle n'en a.
    if missing > parity_in {
        return Err(DnaError::InsufficientData(format!(
            "bloc {block} : {missing} brins sans lecture sur {strands}, pour {parity_in} brins de \
             parité"
        )));
    }
    let decoded = code.decode(&channel, MAX_ITERATIONS)?;
    let mut repaired = 0;
    let mut body = Vec::with_capacity(data_in * PAYLOAD_LEN);
    for strand in 0..strands {
        let changed = soft[first + strand].is_none()
            || (0..PAYLOAD_BITS).any(|bit| {
                decoded.bits[at(strand, bit)] != u8::from(channel[at(strand, bit)] < 0.0)
            });
        repaired += usize::from(changed);
        if strand < data_in {
            for byte in 0..PAYLOAD_LEN {
                body.push(byte_of(
                    (0..8).map(|bit| decoded.bits[at(strand, byte * 8 + bit)]),
                ));
            }
        }
    }
    if !decoded.success {
        return Err(DnaError::InsufficientData(format!(
            "bloc {block} : {data_in} brins de données et {parity_in} de parité, dont \
             {missing} sans lecture sur {read} lus : le code LDPC n'a pas convergé en {} \
             itérations",
            decoded.iterations
        )));
    }
    Ok(BlockRepair {
        body,
        repaired,
        iterations: decoded.iterations,
    })
}

/// Décode le corps de l'archive à partir des lectures (manifestes non compris).
pub fn decode(reads: &[Vec<u8>], body_len: usize, parity: usize) -> Result<(Vec<u8>, DecodeStats)> {
    let layout = LdpcLayout::for_body(body_len, parity)?;
    let total = layout.total_strands as usize;
    let blocks = layout.blocks as usize;
    let mut stats = DecodeStats::default();

    // 1. Le treillis, lecture par lecture : un index, des rapports, un compte de corrections.
    let sized: Vec<&Vec<u8>> = reads
        .iter()
        .filter(|read| read.len() == STRAND_LEN)
        .collect();
    stats.reads_used = sized.len();
    let parsed: Vec<Parsed> = sized.par_iter().map(|read| parse_read(read)).collect();
    stats.corrected = parsed.iter().filter(|parsed| parsed.fixed > 0).count();
    let kept: Vec<&Parsed> = parsed
        .iter()
        .filter(|parsed| (parsed.index as usize) < total)
        .collect();
    stats.uncorrectable = sized.len() - kept.len();

    // 2. Les rapports s'additionnent brin par brin ; un brin sans lecture reste à zéro.
    let mut soft: Vec<Option<Box<[f32]>>> = vec![None; total];
    for parsed in kept {
        let slot = &mut soft[parsed.index as usize];
        match slot {
            None => *slot = Some(parsed.llrs.clone()),
            Some(sum) => {
                for (value, &llr) in sum.iter_mut().zip(&parsed.llrs) {
                    *value += llr;
                }
            }
        }
    }
    stats.strands = soft.iter().flatten().count();

    // 3. Le LDPC, bloc par bloc.
    let full_code = (blocks > 0 && layout.data_in_block(0) == BLOCK_DATA)
        .then(|| block_code(BLOCK_DATA, parity_of(BLOCK_DATA, parity)));
    let last_code = (blocks > 0 && layout.data_in_block(blocks - 1) < BLOCK_DATA).then(|| {
        block_code(
            layout.data_in_block(blocks - 1),
            layout.parity_in_block(blocks - 1),
        )
    });
    let repairs: Vec<Result<BlockRepair>> = (0..blocks)
        .into_par_iter()
        .map(|block| {
            repair_block(
                &layout,
                block,
                &soft,
                full_code.as_ref(),
                last_code.as_ref(),
            )
        })
        .collect();
    let mut body = Vec::with_capacity(layout.data_strands as usize * PAYLOAD_LEN);
    for repair in repairs {
        let repair = repair?;
        stats.blocks_repaired += usize::from(repair.repaired > 0);
        stats.strands_repaired += repair.repaired;
        stats.iterations = stats.iterations.max(repair.iterations);
        body.extend_from_slice(&repair.body);
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
    use crate::codec::rng::SeededRng;

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 5);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    fn encode_of(body: &[u8], parity: usize) -> Vec<Vec<u8>> {
        encode(body, &LdpcParams { parity }, &DnaConstraints::default())
            .unwrap()
            .0
    }

    /// Un canal simple : chaque base lue est fausse avec la probabilité `error`, chaque brin est
    /// lu `coverage` fois en moyenne (loi de Poisson), dans un ordre aléatoire.
    fn channel(oligos: &[Vec<u8>], error: f64, coverage: f64, seed: u64) -> Vec<Vec<u8>> {
        let mut rng = SeededRng::new(seed, 91);
        let mut reads = Vec::new();
        for oligo in oligos {
            // Poisson, par la méthode de Knuth.
            let limit = (-coverage).exp();
            let mut product = 1.0;
            let mut copies = 0;
            loop {
                product *= rng.next_f64().max(f64::MIN_POSITIVE);
                if product <= limit {
                    break;
                }
                copies += 1;
            }
            for _ in 0..copies {
                let read: Vec<u8> = oligo
                    .iter()
                    .map(|&base| {
                        if rng.next_f64() < error {
                            let rank = BASES.iter().position(|&known| known == base).unwrap_or(0);
                            BASES[(rank + 1 + rng.below(3) as usize) % 4]
                        } else {
                            base
                        }
                    })
                    .collect();
                reads.push(read);
            }
        }
        for index in (1..reads.len()).rev() {
            let other = rng.below(index as u64 + 1) as usize;
            reads.swap(index, other);
        }
        reads
    }

    #[test]
    fn a_clean_read_takes_the_fast_path_and_gets_what_the_log_map_says() {
        let oligos = encode_of(&pseudo_random(13 * 30, 17), 20);
        for (index, oligo) in oligos.iter().enumerate() {
            let fast = parse_read(oligo);
            assert_eq!(fast.index as usize, index);
            assert_eq!(fast.fixed, 0);
            assert!(
                fast.llrs.iter().all(|&llr| llr.abs() == CONFIDENCE),
                "brin {index}"
            );
            // Le log-MAP, borné, dit la même chose : ses rapports dépassent la borne.
            let soft = bcjr(&received(oligo), &vec![0.0; INFO_BITS]).unwrap();
            assert!(soft.llrs.iter().all(|&llr| llr.abs() > CONFIDENCE));
            let salt = unwire_salt(
                soft.bits[..SALT_BITS]
                    .iter()
                    .fold(0u32, |salt, &bit| salt << 1 | u32::from(bit)),
            );
            let key = keystream(fast.index, salt);
            for (position, (&llr, &bit)) in fast
                .llrs
                .iter()
                .zip(&soft.llrs[SALT_BITS + INDEX_BITS..])
                .enumerate()
            {
                let mask = (key[position / 8] >> (7 - position % 8)) & 1;
                let bit = bit.clamp(-CONFIDENCE, CONFIDENCE);
                assert_eq!(
                    llr,
                    if mask == 1 { -bit } else { bit },
                    "brin {index}, bit {position}"
                );
            }
        }
        // Une lecture fautive prend le chemin complet, et le treillis la corrige : une base
        // remplacée par sa voisine ne change qu'un bit de code.
        let mut damaged = oligos[3].clone();
        damaged[70] = match damaged[70] {
            b'A' => b'C',
            b'C' => b'A',
            b'G' => b'T',
            _ => b'G',
        };
        let repaired = parse_read(&damaged);
        assert_eq!(repaired.fixed, 1);
        assert_eq!(repaired.index as usize, 3);
        assert_eq!(repaired.llrs, parse_read(&oligos[3]).llrs);
    }

    #[test]
    fn the_channel_model_produces_the_asked_damage() {
        let oligos = encode_of(&pseudo_random(13 * 40, 3), 20);
        let clean = channel(&oligos, 0.0, 3.0, 1);
        // Sans erreur, chaque lecture est un brin de l'archive ; 53 brins, trois lectures en
        // moyenne.
        assert!(clean.iter().all(|read| oligos.contains(read)));
        assert!((120..=200).contains(&clean.len()), "{}", clean.len());
        let damaged = channel(&oligos, 0.02, 3.0, 1);
        let intact = damaged.iter().filter(|read| oligos.contains(read)).count();
        // À 2 % par base sur 154 bases, une lecture sur vingt reste intacte.
        assert!((intact as f64) < 0.1 * damaged.len() as f64, "{intact}");
    }

    #[test]
    fn decodes_under_errors_and_losses() {
        let body = pseudo_random(13 * 120, 11);
        for (parity, error, coverage) in [
            (20, 0.005, 3.0),
            (20, 0.01, 5.0),
            (20, 0.02, 10.0),
            (50, 0.05, 10.0),
        ] {
            let oligos = encode_of(&body, parity);
            let reads = channel(&oligos, error, coverage, 5);
            let (decoded, stats) = decode(&reads, body.len(), parity).unwrap_or_else(|error| {
                panic!("parité {parity}, erreurs {error}, couverture {coverage} : {error}")
            });
            assert_eq!(decoded, body);
            // À couverture 3, 5 % des brins n'ont aucune lecture : le LDPC les restitue. À plus
            // forte couverture, il ne manque forcément rien.
            if coverage == 3.0 {
                assert!(stats.strands_repaired >= 2, "parité {parity} : {stats:?}");
                assert_eq!(stats.blocks_repaired, 1);
            }
            assert!(stats.iterations <= MAX_ITERATIONS);
        }
    }

    #[test]
    fn fails_loudly_when_the_parity_cannot_cover_the_losses() {
        // 120 brins de données et 6 de parité : garder un brin sur sept. Plus de brins manquent
        // que la parité n'en a : refusé d'emblée, sans laisser le code rendre un mot au hasard —
        // un bloc vide, lui, satisferait tous ses contrôles.
        let body = pseudo_random(13 * 120, 13);
        let oligos = encode_of(&body, 5);
        let kept: Vec<Vec<u8>> = oligos.iter().step_by(7).cloned().collect();
        let err = decode(&kept, body.len(), 5).unwrap_err();
        assert!(
            err.to_string()
                .contains("108 brins sans lecture sur 126, pour 6 brins de parité"),
            "{err}"
        );

        // Assez de parité en nombre, pas en pouvoir : à 20 %, 24 brins manquent sur 144, et le
        // code n'en rétablit que 11 %. Soit il ne converge pas, soit il rend le bon mot de code :
        // jamais un autre, que l'enveloppe réfuterait.
        let oligos = encode_of(&body, 20);
        let kept: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(rank, _)| rank % 6 != 0)
            .map(|(_, oligo)| oligo.clone())
            .collect();
        assert!(kept.len() <= 126);
        match decode(&kept, body.len(), 20) {
            Err(error) => assert!(error.to_string().contains("n'a pas convergé"), "{error}"),
            Ok((decoded, _)) => assert_eq!(decoded, body),
        }
    }

    #[test]
    fn the_layout_matches_the_format() {
        assert_eq!(
            (STRAND_LEN, CODED_BITS, INFO_BITS, PAYLOAD_BITS, PAYLOAD_LEN),
            (154, 308, 148, 104, 13)
        );
        assert_eq!(crate::convolutional::coded_len(INFO_BITS), CODED_BITS);
        let empty = LdpcLayout::for_body(0, 20).unwrap();
        assert_eq!(empty.data_strands, 0);
        assert_eq!(empty.total_strands, 0);
        // Un bloc plein : 819 brins de données, 164 de parité à 20 %.
        let full = LdpcLayout::for_body(BLOCK_DATA * PAYLOAD_LEN, 20).unwrap();
        assert_eq!(full.blocks, 1);
        assert_eq!(full.parity_strands, 164);
        assert_eq!(full.total_strands, 983);
        let two = LdpcLayout::for_body(BLOCK_DATA * PAYLOAD_LEN + 1, 20).unwrap();
        assert_eq!(two.blocks, 2);
        assert_eq!(two.data_in_block(1), 1);
        assert_eq!(two.parity_in_block(1), 1);
        assert_eq!(two.first_strand(1), 983);
        assert_eq!(two.total_strands, 983 + 2);
        assert_eq!(parity_of(1000, 20), 200);
        assert_eq!(parity_of(1, 20), 1);
        assert!(LdpcLayout::for_body(0, 4).is_err());
        assert!(LdpcLayout::for_body(0, 101).is_err());
        assert!(LdpcLayout::for_body(usize::MAX, 20).is_err());
        // La limite tient au nombre de brins : 2^24 brins d'index, données et parité, soit
        // environ 180 Mio.
        assert!(LdpcLayout::for_body(190 * 1024 * 1024, 20).is_err());
        assert!(LdpcLayout::for_body(170 * 1024 * 1024, 20).is_ok());
    }

    #[test]
    fn the_salt_is_transmitted_through_a_bijection() {
        let mut seen = vec![false; 1 << SALT_BITS];
        for salt in 0..(1 << SALT_BITS) {
            let wire = wire_salt(salt);
            assert_eq!(unwire_salt(wire), salt);
            assert!(!std::mem::replace(&mut seen[wire as usize], true), "{salt}");
        }
        // Les premiers sels se dispersent : la plupart n'ont pas de zéros en tête, qui
        // s'écriraient en suite de A.
        let spread = (0..64)
            .filter(|&salt| wire_salt(salt) >> (SALT_BITS - 4) != 0)
            .count();
        assert!(spread >= 32, "{spread}");
        // Un index masqué par deux sels différents ne se lit pas de la même façon.
        assert_ne!(index_mask(1), index_mask(2));
        assert_eq!(index_mask(7), index_mask(7));
        assert!(index_mask(7) < 1 << INDEX_BITS);
    }

    #[test]
    fn strands_respect_the_constraints_and_round_trip() {
        let constraints = DnaConstraints::default();
        for (len, parity) in [(1, 20), (13 * 60, 20), (13 * 900, 20), (13 * 300, 50)] {
            let body = pseudo_random(len, 7);
            let (oligos, stats) = encode(&body, &LdpcParams { parity }, &constraints).unwrap();
            let layout = LdpcLayout::for_body(len, parity).unwrap();
            assert_eq!(oligos.len(), layout.total_strands as usize);
            assert_eq!(stats.data_strands, layout.data_strands);
            assert_eq!(stats.parity_strands, layout.parity_strands);
            for oligo in &oligos {
                assert_eq!(oligo.len(), STRAND_LEN);
                constraints.check(oligo).unwrap();
            }
            assert!(
                stats.salts_tried >= oligos.len() as u64,
                "{}",
                stats.salts_tried
            );
            let (decoded, stats) = decode(&oligos, len, parity).unwrap();
            assert_eq!(decoded.len(), body.len(), "{len} octets, parité {parity}");
            assert_eq!(decoded, body, "{len} octets, parité {parity}");
            assert_eq!(stats.reads_used, oligos.len());
            assert_eq!(stats.strands, oligos.len());
            assert_eq!(stats.strands_repaired, 0);
            assert_eq!(stats.blocks_repaired, 0);
        }
    }
}
