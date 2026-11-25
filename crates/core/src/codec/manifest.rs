//! Manifeste : petit en-tête replié dans des oligos dédiés, qui rend une archive lisible sans
//! aucune information extérieure.
//!
//! Un oligo manifeste a la même longueur que les oligos de données du même schéma :
//!
//! | étiquette `ADNF`, 4 octets (16 nt) | nonce, 4 octets (16 nt) | manifeste masqué, 28 octets (112 nt) | parité Reed-Solomon |
//!
//! Le manifeste compte 28 octets :
//!
//! | décalage | taille | contenu |
//! |----------|--------|---------|
//! | 0        | 1      | version du format (2) |
//! | 1        | 1      | schéma : 1 = DNA Fountain, 2 = brins codés, 3 = goldman, 4 = grass, 5 = ldpc, 6 = hedges |
//! | 2        | 1      | parité Reed-Solomon de chaque oligo : en octets (fountain, strands), en symboles (grass), `2` pour goldman et ldpc (leurs manifestes), `0` pour hedges |
//! | 3        | 1      | compression de l'enveloppe |
//! | 4        | 8      | longueur de l'enveloppe (octets, petit-boutiste) |
//! | 12       | 4      | nombre de blocs (DNA Fountain) ou de fenêtres (goldman), 0 sinon |
//! | 16       | 4      | `c` de la distribution soliton (f32, petit-boutiste) ; sinon, au premier octet : identifiant de fichier du schéma goldman, parité du schéma ldpc (en pour cent), rendement du schéma hedges (1 à 4) |
//! | 20       | 4      | `δ` de la distribution soliton (f32, petit-boutiste) |
//! | 24       | 4      | CRC-32 des 24 premiers octets |
//!
//! Le manifeste est masqué par un flux pseudo-aléatoire dérivé du nonce : un contenu très
//! structuré (des octets nuls) donnerait sinon, en ADN, de longues suites de A qu'aucun nonce ne
//! corrigerait. L'étiquette et le nonce restent en clair, pour la reconnaissance et le
//! démasquage ; le nonce est transmis sous une forme dispersée (bijection de 32 bits), pour les
//! mêmes raisons. La parité Reed-Solomon protège l'ensemble (étiquette, nonce, manifeste).
//!
//! L'étiquette permet de reconnaître un oligo manifeste parmi les lectures. Le décodeur retient le
//! manifeste confirmé par le plus grand nombre de lectures.
//!
//! Les archives du schéma hedges font exception : leur oligo manifeste est codé par HEDGES
//! derrière l'étiquette `HEDG` ([`super::hedges`]), pour traverser lui aussi les insertions et
//! les suppressions.

use crate::checksum::crc32;
use crate::codec::rng::{scramble_u32, SeededRng};
use crate::envelope::Compression;
use crate::error::{DnaError, Result};
use crate::rs::{RsCode, MAX_BLOCK_LEN};
use crate::sequence::{bytes_to_dna, dna_to_bytes};
use crate::DnaConstraints;

pub const MANIFEST_TAG: [u8; 4] = *b"ADNF";
pub const NONCE_LEN: usize = 4;
pub const MANIFEST_LEN: usize = 28;
/// Étiquette et nonce, transmis en clair.
pub const CLEAR_LEN: usize = MANIFEST_TAG.len() + NONCE_LEN;
/// Longueur du mot de code : étiquette, nonce et manifeste.
pub const CODEWORD_LEN: usize = CLEAR_LEN + MANIFEST_LEN;
pub const FORMAT_VERSION: u8 = 2;

const MASK_STREAM: u64 = 0x0000_0000_0000_0010;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SchemeId {
    Fountain = 1,
    Strands = 2,
    Goldman = 3,
    Grass = 4,
    Ldpc = 5,
    Hedges = 6,
}

impl SchemeId {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(SchemeId::Fountain),
            2 => Some(SchemeId::Strands),
            3 => Some(SchemeId::Goldman),
            4 => Some(SchemeId::Grass),
            5 => Some(SchemeId::Ldpc),
            6 => Some(SchemeId::Hedges),
            _ => None,
        }
    }
}

/// Paramètres d'une archive, portés par ses oligos manifestes.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub scheme: SchemeId,
    pub rs_parity: u8,
    pub compression: Compression,
    pub body_len: u64,
    pub chunks: u32,
    pub c: f32,
    pub delta: f32,
    /// Identifiant de fichier du schéma goldman (0 à 8), parité du schéma ldpc (5 à 100, pour
    /// cent), rendement du schéma hedges (1 à 4) ; rangé dans le premier octet de `c`, qui
    /// vaut alors 0.
    pub file_id: u8,
}

/// Bilan de la recherche du manifeste dans un ensemble de lectures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ManifestStats {
    /// Lectures dont le début ressemble à l'étiquette.
    pub candidates: usize,
    /// Lectures dont le manifeste a été décodé (parité, étiquette et CRC valides).
    pub valid: usize,
    /// Lectures confirmant le manifeste retenu.
    pub agreeing: usize,
}

/// Sérialise le manifeste.
pub fn encode(manifest: &Manifest) -> [u8; MANIFEST_LEN] {
    let mut bytes = [0u8; MANIFEST_LEN];
    bytes[0] = FORMAT_VERSION;
    bytes[1] = manifest.scheme as u8;
    bytes[2] = manifest.rs_parity;
    bytes[3] = manifest.compression.code();
    bytes[4..12].copy_from_slice(&manifest.body_len.to_le_bytes());
    bytes[12..16].copy_from_slice(&manifest.chunks.to_le_bytes());
    // `c` n'a de sens que pour DNA Fountain : son premier octet porte l'identifiant de fichier
    // du schéma goldman, la parité du schéma ldpc ou le rendement du schéma hedges.
    match manifest.scheme {
        SchemeId::Goldman | SchemeId::Ldpc | SchemeId::Hedges => {
            bytes[16] = manifest.file_id;
            bytes[17..20].fill(0);
        }
        _ => bytes[16..20].copy_from_slice(&manifest.c.to_le_bytes()),
    }
    bytes[20..24].copy_from_slice(&manifest.delta.to_le_bytes());
    let crc = crc32(&bytes[..24]);
    bytes[24..28].copy_from_slice(&crc.to_le_bytes());
    bytes
}

/// Lit et contrôle un manifeste sérialisé.
pub fn decode(bytes: &[u8]) -> Result<Manifest> {
    if bytes.len() != MANIFEST_LEN {
        return Err(DnaError::UnsupportedFormat(format!(
            "manifeste de {} octets au lieu de {MANIFEST_LEN}",
            bytes.len()
        )));
    }
    if bytes[0] != FORMAT_VERSION {
        return Err(DnaError::UnsupportedFormat(format!(
            "manifeste de version {} non pris en charge (attendue: {FORMAT_VERSION})",
            bytes[0]
        )));
    }
    let stored_crc = u32::from_le_bytes(bytes[24..28].try_into().expect("4 octets"));
    if crc32(&bytes[..24]) != stored_crc {
        return Err(DnaError::Integrity(
            "CRC-32 du manifeste incorrect".to_string(),
        ));
    }
    let scheme = SchemeId::from_code(bytes[1])
        .ok_or_else(|| DnaError::UnsupportedFormat(format!("schéma inconnu: {}", bytes[1])))?;
    let compression = Compression::from_code(bytes[3]).ok_or_else(|| {
        DnaError::UnsupportedFormat(format!("compression de manifeste inconnue: {}", bytes[3]))
    })?;
    let (c, file_id) = match scheme {
        SchemeId::Goldman | SchemeId::Ldpc | SchemeId::Hedges => (0.0, bytes[16]),
        _ => (
            f32::from_le_bytes(bytes[16..20].try_into().expect("4 octets")),
            0,
        ),
    };
    Ok(Manifest {
        scheme,
        rs_parity: bytes[2],
        compression,
        body_len: u64::from_le_bytes(bytes[4..12].try_into().expect("8 octets")),
        chunks: u32::from_le_bytes(bytes[12..16].try_into().expect("4 octets")),
        c,
        delta: f32::from_le_bytes(bytes[20..24].try_into().expect("4 octets")),
        file_id,
    })
}

/// Flux de masquage, dérivé du nonce tel qu'il est transmis.
fn mask(wire_nonce: u32, out: &mut [u8; MANIFEST_LEN]) {
    SeededRng::new(u64::from(wire_nonce), MASK_STREAM).fill_bytes(out);
}

/// Applique ou retire le masque sur la zone manifeste du bloc.
fn unmask(block: &mut [u8], wire_nonce: u32) {
    let mut stream = [0u8; MANIFEST_LEN];
    mask(wire_nonce, &mut stream);
    for (slot, &key) in block[CLEAR_LEN..CODEWORD_LEN].iter_mut().zip(stream.iter()) {
        *slot ^= key;
    }
}

/// Octets d'un oligo manifeste avant conversion en ADN : étiquette, nonce, manifeste masqué,
/// parité. La parité est calculée avant masquage.
pub fn oligo_bytes(manifest: &Manifest, nonce: u32, rs: &RsCode) -> Result<Vec<u8>> {
    let mut block = vec![0u8; CODEWORD_LEN + rs.ecc()];
    block[..MANIFEST_TAG.len()].copy_from_slice(&MANIFEST_TAG);
    // Le nonce est transmis sous une forme dispersée : un nonce brut s'écrirait en longues
    // suites de A (octets hauts nuls), que le criblage rejette.
    let wire_nonce = scramble_u32(nonce);
    block[MANIFEST_TAG.len()..CLEAR_LEN].copy_from_slice(&wire_nonce.to_le_bytes());
    block[CLEAR_LEN..CODEWORD_LEN].copy_from_slice(&encode(manifest));
    let (data, parity) = block.split_at_mut(CODEWORD_LEN);
    rs.parity(data, parity)?;
    unmask(&mut block, wire_nonce);
    Ok(block)
}

/// Construit l'oligo (ADN) d'un manifeste pour un nonce donné.
pub fn oligo_for_nonce(manifest: &Manifest, nonce: u32, rs: &RsCode) -> Result<Vec<u8>> {
    let block = oligo_bytes(manifest, nonce, rs)?;
    let mut dna = Vec::with_capacity(block.len() * 4);
    bytes_to_dna(&block, &mut dna);
    Ok(dna)
}

/// Retient, à partir du nonce `nonce_from`, un nonce dont l'oligo respecte les contraintes.
///
/// Le nombre d'essais admis est celui qui laisse moins d'une chance sur 10^17 de n'en pas
/// trouver ; des contraintes que l'espace des nonces ne peut pas satisfaire sont refusées
/// d'emblée. L'étiquette, en clair, écarte un oligo manifeste de l'uniformité : la marge de
/// recherche est donc quadruplée.
pub fn oligo(
    manifest: &Manifest,
    rs: &RsCode,
    constraints: &DnaConstraints,
    nonce_from: u32,
) -> Result<Vec<u8>> {
    let length = (CODEWORD_LEN + rs.ecc()) * 4;
    let space = u64::from(u32::MAX - nonce_from);
    let limit = constraints
        .search_limit(length, space / 4)
        .ok_or_else(|| DnaError::Encoding(constraints.too_strict_message(length)))?;
    let last = nonce_from.saturating_add(u32::try_from(limit * 4).unwrap_or(u32::MAX));
    for nonce in nonce_from..=last {
        let candidate = oligo_for_nonce(manifest, nonce, rs)?;
        if constraints.satisfied(&candidate) {
            return Ok(candidate);
        }
    }
    Err(DnaError::Encoding(
        "aucun oligo manifeste ne satisfait les contraintes: elles sont trop strictes".to_string(),
    ))
}

/// Longueur d'un oligo manifeste dont la parité Reed-Solomon est `rs_parity` octets.
pub fn oligo_length(rs_parity: u8) -> usize {
    (CODEWORD_LEN + usize::from(rs_parity)) * 4
}

/// Étiquette en ADN, pour la reconnaissance rapide.
pub fn tag_dna() -> Vec<u8> {
    let mut dna = Vec::with_capacity(MANIFEST_TAG.len() * 4);
    bytes_to_dna(&MANIFEST_TAG, &mut dna);
    dna
}

/// Vrai si le début de la lecture ressemble à l'étiquette : au plus un groupe de 4 bases
/// différent, c'est-à-dire au plus un octet.
pub fn looks_like_manifest(read: &[u8], tag: &[u8]) -> bool {
    if read.len() < tag.len() {
        return false;
    }
    let mut differing_groups = 0;
    for (group, expected) in read[..tag.len()]
        .as_chunks::<4>()
        .0
        .iter()
        .zip(tag.as_chunks::<4>().0)
    {
        if group != expected {
            differing_groups += 1;
            if differing_groups > 1 {
                return false;
            }
        }
    }
    true
}

/// Décode et corrige le bloc d'un oligo manifeste, pour un nonce transmis donné.
fn parse_block(block: &mut [u8], rs: &RsCode, wire_nonce: u32) -> Result<Manifest> {
    unmask(block, wire_nonce);
    rs.correct(block)?;
    if block[..MANIFEST_TAG.len()] != MANIFEST_TAG {
        return Err(DnaError::UnsupportedFormat(
            "étiquette de manifeste absente".to_string(),
        ));
    }
    decode(&block[CLEAR_LEN..CODEWORD_LEN])
}

fn wire_nonce(block: &[u8]) -> u32 {
    u32::from_le_bytes(
        block[MANIFEST_TAG.len()..CLEAR_LEN]
            .try_into()
            .expect("4 octets"),
    )
}

/// Décode le manifeste porté par une lecture reconnue par son étiquette.
///
/// Une erreur dans le nonce transmis fausse le masque. Si la lecture directe échoue, on essaie
/// chaque variante du nonce à une base près (48 variantes).
pub fn parse_oligo(read: &[u8], rs: &RsCode) -> Result<Manifest> {
    let mut block = dna_to_bytes(read)?;
    if block.len() != CODEWORD_LEN + rs.ecc() {
        return Err(DnaError::UnsupportedFormat(format!(
            "oligo manifeste de {} octets au lieu de {}",
            block.len(),
            CODEWORD_LEN + rs.ecc()
        )));
    }
    let original = block.clone();
    if let Ok(manifest) = parse_block(&mut block, rs, wire_nonce(&original)) {
        return Ok(manifest);
    }

    for byte in MANIFEST_TAG.len()..CLEAR_LEN {
        for field in 0..4 {
            let shift = 6 - 2 * field;
            let current = (original[byte] >> shift) & 3;
            for replacement in 0u8..4 {
                if replacement == current {
                    continue;
                }
                block.copy_from_slice(&original);
                block[byte] = (original[byte] & !(3 << shift)) | replacement << shift;
                let candidate_nonce = wire_nonce(&block);
                if let Ok(manifest) = parse_block(&mut block, rs, candidate_nonce) {
                    return Ok(manifest);
                }
            }
        }
    }
    Err(DnaError::UnsupportedFormat(
        "manifeste illisible : trop d'erreurs dans l'oligo".to_string(),
    ))
}

/// Cherche le manifeste confirmé par le plus de lectures.
///
/// Prend possession des lectures et retourne le manifeste ainsi que les lectures de données
/// (celles qui ne ressemblent pas à un manifeste). Les archives hedges portent leur manifeste
/// derrière l'étiquette `HEDG`, codé par HEDGES : on les reconnaît à leur étiquette, et on essaie
/// leurs rendements ([`super::hedges::probe_manifest_oligo`]).
pub fn find(reads: Vec<Vec<u8>>) -> Result<(Manifest, Vec<Vec<u8>>, ManifestStats)> {
    let tag = tag_dna();
    let hedges_tag = super::hedges::manifest_tag_dna();
    let mut stats = ManifestStats::default();
    let candidates: Vec<&Vec<u8>> = reads
        .iter()
        .filter(|read| looks_like_manifest(read, &tag))
        .collect();
    stats.candidates = candidates.len();

    // La parité se déduit de la longueur de l'oligo manifeste : 36 octets, plus la parité.
    // Un code est construit par longueur rencontrée.
    let mut codes: Vec<(usize, RsCode)> = Vec::new();
    let mut manifests: Vec<Manifest> = Vec::new();
    for read in &candidates {
        let length = read.len();
        if !length.is_multiple_of(4) {
            continue;
        }
        let Some(parity) = (length / 4).checked_sub(CODEWORD_LEN) else {
            continue;
        };
        if parity == 0 || parity >= MAX_BLOCK_LEN {
            continue;
        }
        if !codes.iter().any(|(seen, _)| *seen == length) {
            let Ok(code) = RsCode::new(parity) else {
                continue;
            };
            codes.push((length, code));
        }
        let (_, code) = codes
            .iter()
            .find(|(seen, _)| *seen == length)
            .expect("le code vient d'être inséré");
        if let Ok(manifest) = parse_oligo(read, code) {
            manifests.push(manifest);
        }
    }

    // Les oligos manifestes des archives hedges. Leurs copies sont identiques : une passe
    // courte suffit sur chacune dès qu'une aboutit — une lecture qui résiste coûterait une
    // recherche entière. La recherche entière ne vient que si aucune passe n'a abouti, et sur les
    // premières lectures seulement ([`super::hedges::MAX_SEARCHED_MANIFESTS`]).
    let hedges_candidates: Vec<&[u8]> = reads
        .iter()
        .filter(|read| looks_like_manifest(read, &hedges_tag))
        .map(Vec::as_slice)
        .collect();
    stats.candidates += hedges_candidates.len();
    // Les lectures candidates se lisent en parallèle.
    let mut probed = super::hedges::probe_manifests(&hedges_candidates);
    if probed.is_empty() {
        probed = super::hedges::search_manifests(&hedges_candidates);
    }
    manifests.extend(probed);

    stats.valid = manifests.len();
    if manifests.is_empty() {
        return Err(DnaError::UnsupportedFormat(format!(
            "aucun manifeste lisible parmi {} lectures ({} ressemblaient à un manifeste) : \
             le fichier n'est pas une archive ADN2, ou les erreurs sont trop nombreuses",
            reads.len(),
            stats.candidates
        )));
    }

    // Manifeste majoritaire (comparaison exacte).
    let mut best: Option<(Manifest, usize)> = None;
    for manifest in &manifests {
        let votes = manifests.iter().filter(|other| *other == manifest).count();
        if best.as_ref().is_none_or(|(_, count)| votes > *count) {
            best = Some((manifest.clone(), votes));
        }
    }
    let (manifest, votes) = best.expect("au moins un manifeste");
    stats.agreeing = votes;

    let data_reads: Vec<Vec<u8>> = reads
        .into_iter()
        .filter(|read| !looks_like_manifest(read, &tag) && !looks_like_manifest(read, &hedges_tag))
        .collect();
    Ok((manifest, data_reads, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::goldman;
    use crate::sequence::BASES;

    fn sample() -> Manifest {
        Manifest {
            scheme: SchemeId::Fountain,
            rs_parity: 2,
            compression: Compression::Zstd,
            body_len: 70_001,
            chunks: 2_188,
            c: 0.025,
            delta: 0.001,
            file_id: 0,
        }
    }

    fn random_dna(length: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 4);
        let mut raw = vec![0u8; length];
        rng.fill_bytes(&mut raw);
        raw.iter().map(|&byte| BASES[byte as usize & 3]).collect()
    }

    fn flip(sequence: &mut [u8], position: usize) {
        sequence[position] = match sequence[position] {
            b'A' => b'C',
            _ => b'A',
        };
    }

    #[test]
    fn manifest_roundtrip_and_crc() {
        let manifest = sample();
        let bytes = encode(&manifest);
        assert_eq!(bytes.len(), MANIFEST_LEN);
        assert_eq!(decode(&bytes).unwrap(), manifest);
        let mut corrupted = bytes;
        corrupted[10] ^= 0x01;
        assert!(matches!(decode(&corrupted), Err(DnaError::Integrity(_))));
        corrupted[0] = 3;
        assert!(matches!(
            decode(&corrupted),
            Err(DnaError::UnsupportedFormat(_))
        ));
        assert!(decode(&bytes[..27]).is_err());
    }

    #[test]
    fn manifest_oligo_is_compliant_and_readable() {
        let manifest = sample();
        let rs = RsCode::new(2).unwrap();
        let constraints = DnaConstraints::default();
        let dna = oligo(&manifest, &rs, &constraints, 0).unwrap();
        assert_eq!(dna.len(), 152);
        assert!(constraints.satisfied(&dna));
        // L'étiquette est en clair en tête d'oligo.
        assert_eq!(dna[..16], tag_dna()[..]);
        assert_eq!(parse_oligo(&dna, &rs).unwrap(), manifest);

        // Chaque copie part d'un nonce différent : les séquences diffèrent.
        let other = oligo(&manifest, &rs, &constraints, 997).unwrap();
        assert_ne!(dna, other);
        assert_eq!(parse_oligo(&other, &rs).unwrap(), manifest);
    }

    #[test]
    fn structured_manifests_are_compliant_too() {
        // Longueurs et compteurs nuls : sans masquage, ces octets donnerient de longues suites
        // de A. Avec les deux parités du format (152 et 160 nt).
        let constraints = DnaConstraints::default();
        for (parity, oligo_len) in [(2usize, 152), (4, 160)] {
            let rs = RsCode::new(parity).unwrap();
            for (scheme, body_len, chunks) in [
                (SchemeId::Strands, 0u64, 0u32),
                (SchemeId::Fountain, 60, 2),
                (SchemeId::Fountain, 1 << 24, 524_288),
                (SchemeId::Strands, u64::MAX, 0),
            ] {
                let manifest = Manifest {
                    scheme,
                    rs_parity: parity as u8,
                    compression: Compression::None,
                    body_len,
                    chunks,
                    c: 0.0,
                    delta: 0.0,
                    file_id: 0,
                };
                let dna = oligo(&manifest, &rs, &constraints, 0).unwrap();
                assert_eq!(dna.len(), oligo_len);
                assert!(constraints.satisfied(&dna), "{manifest:?}");
                assert_eq!(parse_oligo(&dna, &rs).unwrap(), manifest);
            }
        }
    }

    #[test]
    fn the_file_id_of_the_goldman_scheme_travels_in_the_manifest() {
        let constraints = DnaConstraints::default();
        for file_id in 0..=8u8 {
            let manifest = Manifest {
                scheme: SchemeId::Goldman,
                rs_parity: 2,
                compression: Compression::None,
                body_len: 3_000,
                chunks: 647,
                c: 0.0,
                delta: 0.0,
                file_id,
            };
            assert_eq!(decode(&encode(&manifest)).unwrap(), manifest);
            let rs = RsCode::new(2).unwrap();
            let dna = oligo(&manifest, &rs, &constraints, 0).unwrap();
            assert!(constraints.satisfied(&dna));
            assert_eq!(parse_oligo(&dna, &rs).unwrap(), manifest);
        }
        // Un identifiant hors limite se transporte tel quel : c'est l'encodage qui le refuse.
        let manifest = decode(&encode(&Manifest {
            scheme: SchemeId::Goldman,
            file_id: 9,
            ..sample()
        }))
        .unwrap();
        assert_eq!(manifest.file_id, 9);
        assert_eq!(manifest.c, 0.0);
        assert!(goldman::GoldmanParams { file_id: 9 }.validate().is_err());
        assert_eq!(decode(&encode(&sample())).unwrap(), sample());
    }

    #[test]
    fn any_single_error_in_a_manifest_oligo_is_corrected() {
        let manifest = sample();
        let rs = RsCode::new(2).unwrap();
        let dna = oligo(&manifest, &rs, &DnaConstraints::default(), 0).unwrap();
        // Étiquette, nonce, manifeste masqué, parité.
        for position in [0usize, 2, 17, 20, 40, 77, 120, 148, 151] {
            let mut damaged = dna.clone();
            flip(&mut damaged, position);
            assert_eq!(
                parse_oligo(&damaged, &rs).unwrap(),
                manifest,
                "position={position}"
            );
        }
    }

    #[test]
    fn heavily_damaged_manifest_is_rejected_or_exact_never_misread() {
        let manifest = sample();
        let rs = RsCode::new(2).unwrap();
        let dna = oligo(&manifest, &rs, &DnaConstraints::default(), 0).unwrap();
        for positions in [[30usize, 80, 130], [5, 45, 90], [17, 19, 140], [0, 4, 8]] {
            let mut damaged = dna.clone();
            for position in positions {
                flip(&mut damaged, position);
            }
            // L'essentiel : jamais un manifeste faux. Une lecture qui ne se corrige pas est
            // refusée, jamais inventée.
            if let Ok(parsed) = parse_oligo(&damaged, &rs) {
                assert_eq!(parsed, manifest, "{positions:?}");
            }
        }
    }

    #[test]
    fn tag_recognition_is_selective() {
        let tag = tag_dna();
        assert_eq!(tag.len(), 16);
        assert!(looks_like_manifest(&tag, &tag));
        let mut near = tag.clone();
        flip(&mut near, 5);
        assert!(looks_like_manifest(&near, &tag));
        let mut far = tag.clone();
        flip(&mut far, 2);
        flip(&mut far, 9);
        assert!(!looks_like_manifest(&far, &tag));
        assert!(!looks_like_manifest(b"AC", &tag));

        // Une lecture aléatoire ressemble à l'étiquette avec une probabilité infime.
        let false_positives = (0..200_000u64)
            .filter(|&seed| looks_like_manifest(&random_dna(16, seed), &tag))
            .count();
        assert!(false_positives <= 1, "{false_positives}");
    }

    #[test]
    fn find_returns_the_majority_manifest_and_the_data_reads() {
        let manifest = sample();
        let rs = RsCode::new(2).unwrap();
        let constraints = DnaConstraints::default();
        let m1 = oligo(&manifest, &rs, &constraints, 0).unwrap();
        let m2 = oligo(&manifest, &rs, &constraints, 997).unwrap();
        // Manifeste illisible : trois erreurs dans trois groupes différents.
        let mut unreadable = m1.clone();
        for position in [20usize, 60, 100] {
            flip(&mut unreadable, position);
        }
        let data_a = random_dna(152, 1);
        let data_b = random_dna(160, 2);
        let reads = vec![data_a.clone(), m1, unreadable, m2, data_b.clone()];
        let (found, data, stats) = find(reads).unwrap();
        assert_eq!(found, manifest);
        assert_eq!(data, vec![data_a, data_b]);
        assert_eq!(stats.candidates, 3);
        assert_eq!(stats.valid, 2);
        assert_eq!(stats.agreeing, 2);
    }

    #[test]
    fn find_ignores_tag_like_reads_of_impossible_lengths() {
        let manifest = sample();
        let rs = RsCode::new(2).unwrap();
        let m = oligo(&manifest, &rs, &DnaConstraints::default(), 0).unwrap();
        let reads = vec![
            m[..151].to_vec(), // longueur non multiple de 4
            m[..143].to_vec(), // moins de 36 octets
            m[..144].to_vec(), // 36 octets, sans parité
            m.clone(),
        ];
        let (found, data, stats) = find(reads).unwrap();
        assert_eq!(found, manifest);
        assert!(data.is_empty());
        assert_eq!(stats.candidates, 4);
        assert_eq!(stats.valid, 1);
    }

    /// Des lectures qui portent l'étiquette `HEDG` sans être des manifestes — un brin tronqué,
    /// par exemple — ne masquent pas les vrais manifestes : la passe courte les écarte, et le
    /// manifeste est trouvé quand bien même il arrive après plus de lectures fausses que la
    /// recherche entière n'en tenterait.
    #[test]
    fn hedges_tag_like_reads_do_not_hide_the_manifest() {
        let options = crate::codec::EncodeOptions {
            compression: Compression::None,
            ..crate::codec::EncodeOptions::for_scheme(crate::codec::Scheme::Hedges)
        };
        let encoded = crate::codec::encode(&[7u8; 300], &options).unwrap();
        let tag = super::super::hedges::manifest_tag_dna();
        let manifests: Vec<Vec<u8>> = encoded
            .oligos
            .iter()
            .filter(|oligo| oligo.starts_with(&tag))
            .cloned()
            .collect();
        assert!(manifests.len() >= 12);
        let data_oligos: Vec<Vec<u8>> = encoded
            .oligos
            .iter()
            .filter(|oligo| !oligo.starts_with(&tag))
            .cloned()
            .collect();
        let mut reads: Vec<Vec<u8>> = (1..=150)
            .map(|seed| {
                let mut read = tag.clone();
                read.extend(random_dna(184, seed));
                read
            })
            .collect();
        reads.extend(manifests.iter().cloned());
        reads.extend(data_oligos.iter().cloned());
        let (found, data, stats) = find(reads).unwrap();
        assert_eq!(found.scheme, SchemeId::Hedges);
        assert_eq!(found.body_len, 360);
        assert_eq!(stats.candidates, 150 + manifests.len());
        assert_eq!(stats.valid, manifests.len());
        assert_eq!(data, data_oligos);
    }

    #[test]
    fn find_fails_loudly_without_manifest() {
        let reads = vec![vec![b'A'; 152], vec![b'C'; 160]];
        assert!(matches!(
            find(reads).unwrap_err(),
            DnaError::UnsupportedFormat(_)
        ));
        assert!(find(Vec::new()).is_err());
    }

    #[test]
    fn impossible_constraints_are_reported() {
        let manifest = sample();
        let rs = RsCode::new(2).unwrap();
        // GC entre 99 % et 100 % : aucun manifeste ne peut passer.
        let constraints = DnaConstraints::new(0.99, 1.0, 3);
        let err = oligo(&manifest, &rs, &constraints, 0).unwrap_err();
        assert!(matches!(err, DnaError::Encoding(_)));
    }
}
