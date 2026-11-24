//! Schéma « hedges » : code à hachis par brin, décodé par recherche, Reed-Solomon entre les
//! brins.
//!
//! Deux codes se succèdent :
//!
//! - **Interne**, brin par brin : HEDGES ([`crate::hedges`]). Il resynchronise chaque lecture —
//!   substitutions, insertions et suppressions — ou déclare forfait. C'est le seul schéma de
//!   l'archive qui traverse les insertions et les suppressions.
//! - **Externe**, bloc par bloc : un Reed-Solomon (255, 223) appliqué « en diagonale ». Le mot de
//!   code `j` prend, du brin `s` du bloc, l'octet de rang `(j + s) mod L`, où `L` est la charge
//!   utile d'un brin. Un brin manquant coûte un octet à chacun des `L` mots de code du bloc : le
//!   bloc retrouve jusqu'à 32 de ses brins, quel que soit leur rang.
//!
//! Un brin porte son index en tête, protégé par le sel : une lecture dont l'index est mal décodé
//! échoue, elle ne se range jamais sous un autre brin. Les lectures d'un même brin doivent
//! concorder ; la moindre divergence écarte le brin, que le code externe restitue.
//!
//! | rendement | motif | brin | bits | charge utile | erreurs par base, des trois sortes |
//! |-----------|-------|------|------|--------------|-------------------------------------|
//! | ¾         | 2, 1  | 200  | 300  | 32 octets    | jusqu'à ~1 %                        |
//! | ½         | 1     | 200  | 200  | 20 octets    | jusqu'à ~3 %                        |
//! | ⅓         | 1, 1, 0 | 300 | 200  | 20 octets    | jusqu'à ~6 %                        |
//! | ¼         | 1, 0  | 300  | 150  | 13 octets    | jusqu'à ~10 %                       |
//!
//! Tous les brins tiennent dans une lecture de 300 bases, celle d'un séquenceur Illumina 2 × 300.
//!
//! Les oligos manifestes sont codés de la même façon, au rendement de l'archive, derrière une
//! étiquette en clair (`HEDG`) et un sel connu : le manifeste traverse lui aussi les insertions et
//! les suppressions.

use crate::checksum::crc32;
use crate::codec::manifest::{Manifest, SchemeId, FORMAT_VERSION};
use crate::envelope::Compression;
use crate::error::{DnaError, Result};
use crate::hedges::{
    self, Decoder, Layout, Outcome, Rate, Search, HEADER_LEN, HEADER_TAG_LEN, SALT_BITS,
};
use crate::rs::RsCode;
use crate::sequence::bytes_to_dna;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

/// Hypothèses au plus par essai de la première passe de lecture d'un manifeste : de quoi lire
/// un oligo intact ou légèrement abîmé, pas de quoi s'épuiser sur un mauvais rendement.
const PROBE_BUDGET: usize = 4096;

/// Lectures candidates au plus que la recherche entière tente quand aucune passe courte n'a
/// abouti : quand tant résistent, elles ne sont sans doute pas des manifestes entiers — des
/// lectures arrêtées avant la fin du brin, par exemple. La recherche du manifeste ne se fait
/// pas en parallèle, et un essai entier coûte quelques millisecondes.
pub const MAX_SEARCHED_MANIFESTS: usize = 128;

/// Brins de données par bloc du code externe.
pub const BLOCK_DATA: usize = 223;
/// Brins de parité par bloc du code externe.
pub const OUTER_PARITY: usize = 32;
/// Brins d'un bloc complet.
pub const BLOCK_STRANDS: usize = BLOCK_DATA + OUTER_PARITY;
/// Nombre total de brins que l'index permet d'adresser.
pub const MAX_STRANDS: u32 = 1 << SALT_BITS;
/// Étiquette en clair d'un oligo manifeste : quatre octets, seize bases.
pub const MANIFEST_TAG: [u8; 4] = *b"HEDG";

/// Paramètres du schéma hedges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HedgesParams {
    /// Rendement du code interne : le motif de bits par base, et la longueur du brin.
    pub rate: Rate,
}

impl Default for HedgesParams {
    fn default() -> Self {
        Self { rate: Rate::HALF }
    }
}

impl HedgesParams {
    /// Vérifie le rendement.
    pub fn validate(&self) -> Result<()> {
        if Rate::ALL.contains(&self.rate) {
            Ok(())
        } else {
            Err(DnaError::InvalidConfig(
                "rendement HEDGES inconnu : choisir 1/4, 1/3, 1/2 ou 3/4".to_string(),
            ))
        }
    }

    /// Longueur d'un brin, en nucléotides.
    pub fn strand_len(&self) -> usize {
        self.rate.chars
    }
}

/// Répartition d'un corps en brins et en blocs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HedgesLayout {
    pub data_strands: u32,
    pub blocks: u32,
    pub total_strands: u32,
    /// Octets de charge utile d'un brin.
    pub payload_len: usize,
}

fn too_large() -> DnaError {
    DnaError::Encoding(format!(
        "fichier trop volumineux pour le schéma hedges (limite : {MAX_STRANDS} brins d'index)"
    ))
}

impl HedgesLayout {
    /// Répartition pour un corps de `body_len` octets, à ce rendement.
    pub fn for_body(body_len: usize, rate: Rate) -> Result<Self> {
        HedgesParams { rate }.validate()?;
        let payload_len = rate.payload_bytes();
        let data_strands =
            u32::try_from(body_len.div_ceil(payload_len)).map_err(|_| too_large())?;
        let blocks = data_strands.div_ceil(BLOCK_DATA as u32);
        let total_strands = data_strands
            .checked_add(blocks * OUTER_PARITY as u32)
            .filter(|&total| total < MAX_STRANDS)
            .ok_or_else(too_large)?;
        Ok(Self {
            data_strands,
            blocks,
            total_strands,
            payload_len,
        })
    }

    /// Brins de données du bloc `block`.
    pub fn data_in_block(&self, block: usize) -> usize {
        (self.data_strands as usize)
            .saturating_sub(block.saturating_mul(BLOCK_DATA))
            .min(BLOCK_DATA)
    }

    /// Brins du bloc `block` : ses données, puis sa parité.
    pub fn strands_in_block(&self, block: usize) -> usize {
        self.data_in_block(block) + OUTER_PARITY
    }

    /// Premier brin du bloc `block`.
    fn first_strand(&self, block: usize) -> usize {
        block * BLOCK_STRANDS
    }
}

/// Bilan d'un encodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeStats {
    pub data_strands: u32,
    pub parity_strands: u32,
}

/// Bilan d'un décodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Lectures de données soumises au code interne.
    pub reads_used: usize,
    /// Lectures que la recherche a corrigées.
    pub corrected: usize,
    /// Lectures irrécupérables, ou d'un index hors de l'archive.
    pub uncorrectable: usize,
    /// Brins connus après le code interne.
    pub strands: usize,
    /// Brins écartés : leurs lectures se partagent entre deux messages, à égalité.
    pub contested: usize,
    /// Lectures dont le message a été écarté : les autres lectures du brin en ont rendu un autre,
    /// plus souvent.
    pub outvoted: usize,
    /// Blocs où le code externe a restitué ou corrigé un brin.
    pub blocks_repaired: usize,
    /// Brins restitués ou corrigés par le code externe.
    pub strands_repaired: usize,
}

/// Les octets libres d'un brin : son index sur trois octets (les trois de poids faible d'un mot
/// de 32 bits), puis sa charge utile.
fn strand_message(index: usize, payload: &[u8]) -> Vec<u8> {
    let mut free = Vec::with_capacity(3 + payload.len());
    free.extend_from_slice(&(index as u32).to_be_bytes()[1..]);
    free.extend_from_slice(payload);
    free
}

/// Encode le corps d'une archive en brins (manifestes non compris).
pub fn encode(body: &[u8], params: &HedgesParams) -> Result<(Vec<Vec<u8>>, EncodeStats)> {
    let layout = HedgesLayout::for_body(body.len(), params.rate)?;
    let strand = Layout::strand(params.rate);
    let payload_len = layout.payload_len;
    let mut payloads = vec![vec![0u8; payload_len]; layout.total_strands as usize];
    // Chaque brin de données prend rang dans son bloc, après la parité du bloc précédent.
    for (number, chunk) in body.chunks(payload_len).enumerate() {
        let (block, position) = (number / BLOCK_DATA, number % BLOCK_DATA);
        payloads[layout.first_strand(block) + position][..chunk.len()].copy_from_slice(chunk);
    }

    // Code externe : la parité de chaque bloc, octet par octet, en diagonale.
    let rs = RsCode::new(OUTER_PARITY)?;
    let mut symbols = vec![0u8; BLOCK_DATA];
    let mut parity = vec![0u8; OUTER_PARITY];
    for block in 0..layout.blocks as usize {
        let (data, first) = (layout.data_in_block(block), layout.first_strand(block));
        for byte in 0..payload_len {
            for strand in 0..data {
                symbols[strand] = payloads[first + strand][(byte + strand) % payload_len];
            }
            rs.parity(&symbols[..data], &mut parity)?;
            // La diagonale court sur tous les brins du bloc, parité comprise.
            for (rank, (slot, &value)) in payloads[first + data..first + data + OUTER_PARITY]
                .iter_mut()
                .zip(parity.iter())
                .enumerate()
            {
                slot[(byte + data + rank) % payload_len] = value;
            }
        }
    }

    let oligos: Vec<Vec<u8>> = payloads
        .par_iter()
        .enumerate()
        .map(|(index, payload)| hedges::encode(&strand, &strand_message(index, payload)))
        .collect();
    let stats = EncodeStats {
        data_strands: layout.data_strands,
        parity_strands: layout.blocks * OUTER_PARITY as u32,
    };
    Ok((oligos, stats))
}

/// Ce qu'un bloc rend : ses brins changés, et combien.
struct BlockRepair {
    /// Brins restitués ou corrigés, avec leur rang dans l'archive.
    changed: Vec<(usize, Vec<u8>)>,
}

/// Applique le code externe à un bloc : chaque octet d'un brin appartient à un mot de code.
///
/// Un brin manquant y laisse un effacement, un brin faux une erreur. Les effacements sont
/// repérés ; le Reed-Solomon corrige jusqu'à 32 effacements, ou 16 erreurs, ou un mélange — une
/// erreur compte double.
fn repair_block(
    layout: &HedgesLayout,
    rs: &RsCode,
    block: usize,
    known: &[Option<Box<[u8]>>],
) -> Result<BlockRepair> {
    let (data, first) = (layout.data_in_block(block), layout.first_strand(block));
    let strands = data + OUTER_PARITY;
    let payload_len = layout.payload_len;
    // La parité ne comblera jamais plus de brins qu'elle n'en a.
    let missing: Vec<usize> = (0..strands)
        .filter(|&strand| known[first + strand].is_none())
        .collect();
    if missing.len() > OUTER_PARITY {
        return Err(DnaError::InsufficientData(format!(
            "bloc {block} : {} brins sans lecture sur {strands}, pour {OUTER_PARITY} brins de \
             parité",
            missing.len()
        )));
    }
    // La charge utile des brins changés, telle que le code la rétablit.
    let mut changed: Vec<Option<Vec<u8>>> = (0..strands).map(|_| None).collect();
    let (mut word, mut missing) = (vec![0u8; strands], Vec::new());
    for byte in 0..payload_len {
        missing.clear();
        for (strand, slot) in word.iter_mut().enumerate() {
            match &known[first + strand] {
                Some(payload) => *slot = payload[(byte + strand) % payload_len],
                None => {
                    *slot = 0;
                    missing.push(strand);
                }
            }
        }
        let before = word.clone();
        if missing.is_empty() {
            if rs.check(&word) {
                continue;
            }
            rs.correct(&mut word)?;
        } else {
            rs.correct_with_erasures(&mut word, &missing)?;
        }
        for strand in 0..strands {
            if word[strand] != before[strand] || missing.contains(&strand) {
                let payload = changed[strand].get_or_insert_with(|| {
                    known[first + strand]
                        .as_deref()
                        .map_or(vec![0u8; payload_len], <[u8]>::to_vec)
                });
                payload[(byte + strand) % payload_len] = word[strand];
            }
        }
    }
    Ok(BlockRepair {
        changed: changed
            .into_iter()
            .enumerate()
            .filter_map(|(strand, bytes)| bytes.map(|bytes| (first + strand, bytes)))
            .collect(),
    })
}

/// Décode le corps de l'archive à partir des lectures (manifestes non compris).
pub fn decode(reads: &[Vec<u8>], body_len: usize, rate: Rate) -> Result<(Vec<u8>, DecodeStats)> {
    let layout = HedgesLayout::for_body(body_len, rate)?;
    let total = layout.total_strands as usize;
    let payload_len = layout.payload_len;
    let mut stats = DecodeStats {
        reads_used: reads.len(),
        ..DecodeStats::default()
    };

    // 1. Le code interne, lecture par lecture : un index, une charge utile, un compte d'erreurs.
    let solved: Vec<(u32, Box<[u8]>, u16)> = reads
        .par_iter()
        .map_init(
            || Decoder::new(Layout::strand(rate)),
            |decoder, read| {
                let decoded = decoder.decode(read);
                if decoded.outcome != Outcome::Solved {
                    return None;
                }
                let message = decoded.message?;
                let index = u32::from(message[0]) << 16
                    | u32::from(message[1]) << 8
                    | u32::from(message[2]);
                Some((index, message[3..].into(), decoded.errors))
            },
        )
        .flatten()
        .collect();

    // 2. Brin par brin : les lectures votent. Un index hors de l'archive est perdu.
    let mut known: Vec<Option<Box<[u8]>>> = vec![None; total];
    let mut votes = vec![0usize; total];
    // L'autre message en lice pour un brin, et ses voix.
    let mut rivals: HashMap<usize, (Box<[u8]>, usize)> = HashMap::new();
    let mut contested = vec![false; total];
    stats.corrected = solved.iter().filter(|(_, _, errors)| *errors > 0).count();
    let mut kept = 0usize;
    for (index, payload, _) in solved {
        let strand = index as usize;
        if strand >= total {
            stats.uncorrectable += 1;
            continue;
        }
        kept += 1;
        if let Some(leading) = &known[strand] {
            if leading.as_ref() == payload.as_ref() {
                votes[strand] += 1;
                continue;
            }
            match rivals.entry(strand) {
                Entry::Vacant(entry) => {
                    entry.insert((payload, 1));
                }
                Entry::Occupied(mut entry) => {
                    if entry.get().0.as_ref() == payload.as_ref() {
                        entry.get_mut().1 += 1;
                    }
                    // Un troisième message : on ne choisit pas.
                    else {
                        contested[strand] = true;
                    }
                }
            };
            continue;
        }
        known[strand] = Some(payload);
        votes[strand] = 1;
    }
    stats.uncorrectable += reads.len() - kept;
    // Les voix comptent : la charge utile la plus votée l'emporte ; à égalité, le brin est écarté.
    for (strand, (rival, rival_votes)) in rivals {
        match rival_votes.cmp(&votes[strand]) {
            Ordering::Greater => {
                stats.outvoted += votes[strand];
                known[strand] = Some(rival);
                votes[strand] = rival_votes;
            }
            Ordering::Less => stats.outvoted += rival_votes,
            Ordering::Equal => contested[strand] = true,
        }
    }
    for (strand, slot) in known.iter_mut().enumerate() {
        if contested[strand] {
            *slot = None;
        }
    }
    stats.contested = contested.iter().filter(|&&yes| yes).count();
    stats.strands = known.iter().flatten().count();

    // 3. Le code externe, bloc par bloc.
    let rs = RsCode::new(OUTER_PARITY)?;
    let repairs: Vec<Result<BlockRepair>> = (0..layout.blocks as usize)
        .into_par_iter()
        .map(|block| repair_block(&layout, &rs, block, &known))
        .collect();
    for repair in repairs {
        let repair = repair?;
        stats.blocks_repaired += usize::from(!repair.changed.is_empty());
        stats.strands_repaired += repair.changed.len();
        for (index, bytes) in repair.changed {
            known[index] = Some(bytes.into_boxed_slice());
        }
    }

    // 4. Le corps : les brins de données de chaque bloc, dans l'ordre, la parité reste à part.
    let mut body = Vec::with_capacity(layout.data_strands as usize * payload_len);
    let mut missing = Vec::new();
    for block in 0..layout.blocks as usize {
        let (data, first) = (layout.data_in_block(block), layout.first_strand(block));
        for strand in 0..data {
            match &known[first + strand] {
                Some(payload) => body.extend_from_slice(payload),
                None => missing.push(first + strand),
            }
        }
    }
    if !missing.is_empty() {
        return Err(DnaError::InsufficientData(format!(
            "{} brins de données manquent (les premiers : {:?}), {} sont écartés pour désaccord, \
             sur {} : le code externe restitue {OUTER_PARITY} brins par bloc de {BLOCK_STRANDS}",
            missing.len(),
            &missing[..missing.len().min(5)],
            stats.contested,
            layout.data_strands
        )));
    }
    body.truncate(body_len);
    Ok((body, stats))
}

/// L'oligo manifeste d'une archive hedges : l'étiquette en clair, puis le manifeste codé par
/// HEDGES au rendement de l'archive, derrière un sel connu.
///
/// Il a la longueur d'un brin de données. Le manifeste tient en quatorze octets : la version
/// du format, le schéma, le rendement, la compression, la longueur du corps sur cinq octets —
/// un téraoctet, pour une limite de 182 Mio au plus —, le rang de la copie, et un CRC-32.
pub fn manifest_oligo(manifest: &Manifest, copy: u16) -> Vec<u8> {
    let rate = rate_of(manifest).expect("le manifeste d'une archive hedges porte son rendement");
    assert!(
        manifest.body_len < 1 << 40,
        "la longueur du corps ne tient pas sur cinq octets"
    );
    let copy = u8::try_from(copy).expect("au plus 256 copies du manifeste");
    let mut content = [0u8; HEADER_LEN];
    content[0] = FORMAT_VERSION;
    content[1] = SchemeId::Hedges as u8;
    content[2] = rate.code();
    content[3] = manifest.compression.code();
    content[4..9].copy_from_slice(&manifest.body_len.to_le_bytes()[..5]);
    content[9] = copy;
    let crc = crc32(&content[..10]);
    content[10..14].copy_from_slice(&crc.to_le_bytes());
    let mut oligo = manifest_tag_dna();
    oligo.extend(hedges::encode(&Layout::header(rate), &content));
    oligo
}

/// L'étiquette des oligos manifestes d'une archive hedges, en ADN.
pub fn manifest_tag_dna() -> Vec<u8> {
    let mut dna = Vec::with_capacity(HEADER_TAG_LEN);
    bytes_to_dna(&MANIFEST_TAG, &mut dna);
    dna
}

/// Le rendement d'une archive hedges, rangé dans le manifeste.
pub fn rate_of(manifest: &Manifest) -> Option<Rate> {
    Rate::from_code(manifest.file_id)
}

/// Lit un manifeste dans les quatorze octets que rend le décodage.
fn header_manifest(rate: Rate, content: &[u8]) -> Option<Manifest> {
    if content[0] != FORMAT_VERSION
        || content[1] != SchemeId::Hedges as u8
        || content[2] != rate.code()
        || crc32(&content[..10]).to_le_bytes() != content[10..14]
    {
        return None;
    }
    let compression = Compression::from_code(content[3])?;
    let mut length = [0u8; 8];
    length[..5].copy_from_slice(&content[4..9]);
    Some(Manifest {
        scheme: SchemeId::Hedges,
        rs_parity: 0,
        compression,
        body_len: u64::from_le_bytes(length),
        chunks: 0,
        c: 0.0,
        delta: 0.0,
        file_id: rate.code(),
    })
}

/// Cherche un manifeste dans une lecture : à chaque début possible du code, à chaque rendement.
fn search_manifest_oligo(read: &[u8], search: Search) -> Option<Manifest> {
    for start in [HEADER_TAG_LEN, HEADER_TAG_LEN - 1, HEADER_TAG_LEN + 1] {
        let Some(code) = read.get(start..) else {
            continue;
        };
        for rate in Rate::ALL {
            let decoded = Decoder::with_search(Layout::header(rate), search).decode(code);
            if decoded.outcome != Outcome::Solved {
                continue;
            }
            let manifest = decoded
                .message
                .as_deref()
                .and_then(|content| header_manifest(rate, content));
            if manifest.is_some() {
                return manifest;
            }
        }
    }
    None
}

/// Lit un manifeste par la seule passe courte d'une lecture : de quoi lire un oligo intact ou
/// légèrement abîmé, pas de quoi s'épuiser sur une lecture qui n'en est pas une. Les copies
/// d'un manifeste sont toutes identiques : il suffit qu'une passe aboutisse, et les lectures
/// qui résistent sont alors laissées là ([`super::manifest::find`]).
pub fn probe_manifest_oligo(read: &[u8]) -> Option<Manifest> {
    search_manifest_oligo(
        read,
        Search {
            budget: PROBE_BUDGET,
            ..Search::default()
        },
    )
}

/// Décode l'oligo manifeste d'une archive hedges.
///
/// Le rendement n'est pas connu d'avance : les quatre sont essayés, à chaque début possible du
/// code. Une base perdue ou ajoutée dans l'étiquette est rattrapée — le code commence alors une
/// base plus tôt ou plus tard. Les rendements d'une autre longueur de brin sont refusés d'emblée
/// par la recherche : deux seulement sont cherchés, et une recherche qui échoue va au bout de
/// son budget. Une première passe courte les fait bon marché, et une lecture intacte ou
/// légèrement abîmée y suffit ; la recherche entière ne vient que si rien n'a abouti.
pub fn parse_manifest_oligo(read: &[u8]) -> Result<Manifest> {
    let full = Search::default();
    probe_manifest_oligo(read)
        .or_else(|| {
            (full.budget > PROBE_BUDGET)
                .then(|| search_manifest_oligo(read, full))
                .flatten()
        })
        .ok_or_else(|| {
            DnaError::UnsupportedFormat(
                "oligo manifeste hedges illisible : trop d'erreurs dans la lecture".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::manifest;
    use crate::codec::rng::SeededRng;
    use crate::constraints::DnaConstraints;
    use crate::sequence::BASES;

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 5);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    fn encode_of(body: &[u8], rate: Rate) -> Vec<Vec<u8>> {
        encode(body, &HedgesParams { rate }).unwrap().0
    }

    /// Un brin forgé : l'index donné, la charge utile donnée.
    fn forge(index: usize, payload: &[u8], rate: Rate) -> Vec<u8> {
        hedges::encode(&Layout::strand(rate), &strand_message(index, payload))
    }

    /// La charge utile d'un brin, lue par le code interne.
    fn payload_of(oligo: &[u8], rate: Rate) -> Vec<u8> {
        let decoded = Decoder::new(Layout::strand(rate)).decode(oligo);
        decoded.message.unwrap()[3..].to_vec()
    }

    /// Un canal : substitutions, insertions et suppressions, `each` chacune ; un brin est lu
    /// `coverage` fois en moyenne (loi de Poisson), dans un ordre aléatoire.
    fn channel(oligos: &[Vec<u8>], each: f64, coverage: f64, seed: u64) -> Vec<Vec<u8>> {
        let mut rng = SeededRng::new(seed, 91);
        let mut reads = Vec::new();
        for oligo in oligos {
            let limit = (-coverage).exp();
            let (mut product, mut copies) = (1.0, 0);
            loop {
                product *= rng.next_f64().max(f64::MIN_POSITIVE);
                if product <= limit {
                    break;
                }
                copies += 1;
            }
            for _ in 0..copies {
                let mut read = Vec::with_capacity(oligo.len());
                for &base in oligo {
                    let roll = rng.next_f64();
                    let rank = BASES.iter().position(|&known| known == base).unwrap_or(0);
                    if roll < each {
                        read.push(BASES[(rank + 1 + rng.below(3) as usize) % 4]);
                    } else if roll < 2.0 * each {
                        read.push(BASES[rng.below(4) as usize]);
                        read.push(base);
                    } else if roll < 3.0 * each {
                        // La base disparaît.
                    } else {
                        read.push(base);
                    }
                }
                reads.push(read);
            }
        }
        for index in (1..reads.len()).rev() {
            let other = rng.below(index as u64 + 1) as usize;
            reads.swap(index, other);
        }
        reads
    }

    /// Une archive et son découpage, à ce rendement.
    fn archive(rate: Rate, data_len: usize, seed: u64) -> (Vec<u8>, Vec<Vec<u8>>, HedgesLayout) {
        let body = pseudo_random(data_len, seed);
        let layout = HedgesLayout::for_body(body.len(), rate).unwrap();
        let oligos = encode_of(&body, rate);
        (body, oligos, layout)
    }

    fn manifest_of(rate: Rate) -> Manifest {
        Manifest {
            scheme: SchemeId::Hedges,
            rs_parity: 0,
            compression: Compression::Zstd,
            body_len: 100_000,
            chunks: 0,
            c: 0.0,
            delta: 0.0,
            file_id: rate.code(),
        }
    }

    #[test]
    fn the_layout_follows_the_blocks() {
        for rate in Rate::ALL {
            let len = rate.payload_bytes();
            let empty = HedgesLayout::for_body(0, rate).unwrap();
            assert_eq!(empty.total_strands, 0);
            let one = HedgesLayout::for_body(1, rate).unwrap();
            assert_eq!(one.data_strands, 1);
            assert_eq!(one.blocks, 1);
            assert_eq!(one.total_strands, 1 + OUTER_PARITY as u32);
            let full = HedgesLayout::for_body(BLOCK_DATA * len, rate).unwrap();
            assert_eq!(full.blocks, 1);
            assert_eq!(full.total_strands as usize, BLOCK_STRANDS);
            let two = HedgesLayout::for_body(BLOCK_DATA * len + 1, rate).unwrap();
            assert_eq!(two.blocks, 2);
            assert_eq!(two.total_strands as usize, BLOCK_STRANDS + 1 + OUTER_PARITY);
            assert_eq!(two.data_in_block(1), 1);
            assert_eq!(two.strands_in_block(1), 1 + OUTER_PARITY);
            assert!(HedgesLayout::for_body(usize::MAX, rate).is_err());
        }
        assert!(HedgesLayout::for_body(180 * 1024 * 1024, Rate::QUARTER).is_ok());
        assert!(HedgesLayout::for_body(182 * 1024 * 1024, Rate::QUARTER).is_err());
        assert!(HedgesLayout::for_body(190 * 1024 * 1024, Rate::HALF).is_ok());
        assert!(HedgesParams::validate(&HedgesParams::default()).is_ok());
    }

    #[test]
    fn strands_carry_their_index_and_respect_the_constraints() {
        let constraints = DnaConstraints::default();
        for rate in Rate::ALL {
            let (body, oligos, layout) = archive(rate, 25 * rate.payload_bytes(), 3);
            assert_eq!(body.len(), 25 * rate.payload_bytes());
            assert_eq!(oligos.len(), layout.total_strands as usize);
            let mut decoder = Decoder::new(Layout::strand(rate));
            for (index, oligo) in oligos.iter().enumerate() {
                assert_eq!(oligo.len(), rate.chars);
                constraints.check(oligo).unwrap();
                let decoded = decoder.decode(oligo);
                assert_eq!(decoded.outcome, Outcome::Solved);
                let message = decoded.message.unwrap();
                assert_eq!(message.len(), 3 + rate.payload_bytes());
                assert_eq!(&message[..3], &(index as u32).to_be_bytes()[1..]);
                if index % 25 == 0 && index != 0 {
                    assert_ne!(&message[..3], &[0, 0, 0], "brin {index} : index tronqué");
                }
            }
        }
    }

    #[test]
    fn decodes_under_indels_and_losses() {
        // (rendement, erreurs de chaque sorte par base, couverture). Aux taux nominaux de
        // l'article : ~1 % au total à ¾, 3 % à ½, 6 % à ⅓, 10 % à ¼.
        for (rate, each, coverage) in [
            (Rate::THREE_QUARTERS, 0.003, 3.0),
            (Rate::HALF, 0.01, 3.0),
            (Rate::THIRD, 0.02, 3.0),
            (Rate::QUARTER, 0.033, 3.0),
        ] {
            let (body, oligos, _) = archive(rate, 30 * rate.payload_bytes(), 11);
            let reads = channel(&oligos, each, coverage, 5);
            let (decoded, stats) = decode(&reads, body.len(), rate).unwrap_or_else(|error| {
                panic!(
                    "rendement {}, erreurs {each}, couverture {coverage} : {error}",
                    rate.name()
                )
            });
            assert_eq!(decoded, body, "rendement {}", rate.name());
            assert_eq!(stats.reads_used, reads.len());
            // À ¼ et 10 % d'erreurs, une lecture faux sur quelques centaines passe encore : le
            // vote ou le code externe l'arrêtent.
            assert!(
                stats.contested + stats.outvoted <= oligos.len() / 10,
                "rendement {} : {} brins écartés, {} lectures écartées",
                rate.name(),
                stats.contested,
                stats.outvoted
            );
            // Le canal retire des brins entiers (loi de Poisson) : le code externe les rend.
            assert!(stats.strands + stats.strands_repaired >= oligos.len());
        }
    }

    #[test]
    fn the_outer_code_restores_missing_strands() {
        let rate = Rate::HALF;
        // Deux cent trente brins de données : un bloc plein, un bloc court.
        let (body, oligos, layout) = archive(rate, 230 * rate.payload_bytes(), 17);
        assert_eq!(layout.blocks, 2);
        assert_eq!(oligos.len(), 230 + 2 * OUTER_PARITY);

        // À couverture 1, sans erreurs : trente-deux brins perdus dans le premier bloc, données
        // et parité confondues — tout ce que le code en retrouvait.
        let lost: Vec<usize> = (0..OUTER_PARITY).map(|rank| rank * 8).collect();
        let reads: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| !lost.contains(index))
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let (decoded, stats) = decode(&reads, body.len(), rate).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.strands, oligos.len() - lost.len());
        assert_eq!(stats.blocks_repaired, 1);
        assert_eq!(stats.strands_repaired, lost.len());

        // Un de plus : l'échec est annoncé.
        let mut lost = lost;
        lost.push(1);
        let reads: Vec<Vec<u8>> = oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| !lost.contains(index))
            .map(|(_, oligo)| oligo.clone())
            .collect();
        let error = decode(&reads, body.len(), rate).unwrap_err();
        assert!(matches!(error, DnaError::InsufficientData(_)), "{error}");
    }

    #[test]
    fn the_reads_of_a_strand_vote() {
        let rate = Rate::THIRD;
        let (body, oligos, _) = archive(rate, 20 * rate.payload_bytes(), 19);
        let (_, other, _) = archive(rate, 20 * rate.payload_bytes(), 23);
        let forged = forge(40, &payload_of(&other[41], rate), rate);

        // Deux lectures, deux messages : on ne choisit pas. Le brin est écarté, et le code
        // externe le restitue.
        let mut reads = oligos.clone();
        reads.push(forged.clone());
        let (decoded, stats) = decode(&reads, body.len(), rate).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 1);
        assert_eq!(stats.outvoted, 0);
        assert_eq!(stats.strands, oligos.len() - 1);
        assert_eq!(stats.strands_repaired, 1);

        // Trois lectures contre une : la charge utile la plus votée l'emporte.
        let mut reads = oligos.clone();
        reads.extend([forged, oligos[40].clone(), oligos[40].clone()]);
        let (decoded, stats) = decode(&reads, body.len(), rate).unwrap();
        assert_eq!(decoded, body);
        assert_eq!(stats.contested, 0);
        assert_eq!(stats.outvoted, 1);
        assert_eq!(stats.strands, oligos.len());
        assert_eq!(stats.strands_repaired, 0);

        // Des lectures qui concordent ne font pas un désaccord.
        let mut reads = oligos.clone();
        reads.push(oligos[40].clone());
        let (_, stats) = decode(&reads, body.len(), rate).unwrap();
        assert_eq!(stats.contested, 0);
        assert_eq!(stats.outvoted, 0);
        assert_eq!(stats.strands, oligos.len());
    }

    #[test]
    fn false_strands_are_corrected_up_to_sixteen_per_block() {
        let rate = Rate::HALF;
        let (body, oligos, layout) = archive(rate, 40 * rate.payload_bytes(), 29);
        assert_eq!(layout.blocks, 1);
        let wrong = pseudo_random(rate.payload_bytes(), 31);
        // Des brins faux, index intact, sans lecture qui les contredise (couverture 1).
        for count in [1usize, 8, 16] {
            let mut reads = oligos.clone();
            for (index, read) in reads.iter_mut().enumerate().take(count) {
                *read = forge(index, &wrong, rate);
            }
            let (decoded, stats) = decode(&reads, body.len(), rate).unwrap();
            assert_eq!(decoded, body, "{count} brins faux");
            assert_eq!(stats.strands_repaired, count, "{count} brins faux");
        }
        // Dix-sept : le code ne peut plus, l'échec est annoncé — jamais un corps faux.
        let mut reads = oligos.clone();
        for (index, read) in reads.iter_mut().enumerate().take(17) {
            *read = forge(index, &wrong, rate);
        }
        let error = decode(&reads, body.len(), rate).unwrap_err();
        assert!(matches!(error, DnaError::Correction(_)), "{error}");
    }

    #[test]
    fn the_manifest_oligo_resists_indels() {
        for rate in Rate::ALL {
            let manifest = manifest_of(rate);
            let oligo = manifest_oligo(&manifest, 7);
            assert_eq!(oligo.len(), rate.chars);
            assert_eq!(oligo[..HEADER_TAG_LEN], manifest_tag_dna()[..]);
            DnaConstraints::default().check(&oligo).unwrap();
            assert_eq!(parse_manifest_oligo(&oligo).unwrap(), manifest);

            // Abîmé : substitutions, insertions et suppressions, des trois sortes.
            let each = if rate == Rate::THREE_QUARTERS {
                0.003
            } else {
                0.01
            };
            let reads = channel(&[oligo], each, 8.0, 13);
            let parsed = reads
                .iter()
                .filter(|read| !read.is_empty())
                .filter_map(|read| parse_manifest_oligo(read).ok())
                .count();
            assert!(
                parsed >= 2,
                "rendement {} : {parsed} manifestes lus",
                rate.name()
            );

            // Une base perdue, ou une de trop, dans l'étiquette.
            let mut shorter = manifest_oligo(&manifest, 9);
            shorter.remove(5);
            assert_eq!(parse_manifest_oligo(&shorter).unwrap(), manifest);
            let mut longer = manifest_oligo(&manifest, 9);
            longer.insert(3, b'T');
            assert_eq!(parse_manifest_oligo(&longer).unwrap(), manifest);
        }
    }

    #[test]
    fn a_foreign_read_is_not_a_manifest() {
        let manifest = manifest_of(Rate::HALF);
        let oligo = manifest_oligo(&manifest, 1);
        // Un brin de données ; l'étiquette d'un manifeste d'un autre schéma.
        let strands = encode_of(&pseudo_random(200, 1), Rate::HALF);
        assert!(parse_manifest_oligo(&strands[0]).is_err());
        assert!(parse_manifest_oligo(&manifest::tag_dna()).is_err());
        // Les deux étiquettes diffèrent par leurs quatre octets : aucune lecture ne ressemble aux
        // deux à la fois.
        let tag = manifest_tag_dna();
        assert_ne!(tag, manifest::tag_dna());
        for base in *b"ACGT" {
            let mut near = tag.clone();
            near[0] = base;
            assert!(!manifest::looks_like_manifest(&near, &manifest::tag_dna()));
        }
        // Un octet du manifeste altéré : le CRC-32 ne passe pas.
        let mut forged = oligo.clone();
        forged[HEADER_TAG_LEN + 40] = match forged[HEADER_TAG_LEN + 40] {
            b'A' => b'G',
            _ => b'A',
        };
        assert!(
            parse_manifest_oligo(&forged).is_err()
                || parse_manifest_oligo(&forged).unwrap() == manifest,
            "une altération ne rend jamais un autre manifeste"
        );
    }
}
