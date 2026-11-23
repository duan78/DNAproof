//! Enveloppe de fichier : compression et vérification d'intégrité.
//!
//! L'enveloppe est le corps du fichier, tel qu'il est réparti dans les oligos. Elle s'écrit :
//!
//! | décalage | taille | contenu |
//! |----------|--------|---------|
//! | 0        | 4      | magic `ADN2` |
//! | 4        | 1      | version du format (2) |
//! | 5        | 1      | compression : 0 aucune, 1 LZ4, 2 zstd |
//! | 6        | 2      | réservé (0) |
//! | 8        | 8      | longueur du fichier original |
//! | 16       | 8      | longueur des données stockées |
//! | 24       | 32     | SHA-256 du fichier original |
//! | 56       | 4      | CRC-32 de l'en-tête suivi des données stockées |
//! | 60       | …      | données stockées (compressées ou non) |
//!
//! Aucun fichier n'est rendu si l'une des vérifications échoue : c'est la garantie qu'une
//! archive abîmée produit une erreur, jamais un contenu faux.

use crate::checksum::{crc32_parts, sha256};
use crate::error::{DnaError, Result};
use std::io::Read;

pub const MAGIC: [u8; 4] = *b"ADN2";
pub const VERSION: u8 = 2;
pub const HEADER_LEN: usize = 60;

/// Niveau de compression zstd (3 : compromis rapide de la bibliothèque).
const ZSTD_LEVEL: i32 = 3;

/// Méthode de compression de l'enveloppe. `Auto` ne sert qu'à l'encodage : elle retient zstd
/// si la compression rapporte, et aucune sinon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Compression {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
    Auto = 255,
}

impl Compression {
    /// Code stocké dans l'enveloppe.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Nom affichable.
    pub fn name(self) -> &'static str {
        match self {
            Compression::None => "aucune",
            Compression::Lz4 => "LZ4",
            Compression::Zstd => "zstd",
            Compression::Auto => "auto",
        }
    }

    /// Retrouve une méthode depuis son code stocké (`Auto` n'est jamais stocké).
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Compression::None),
            1 => Some(Compression::Lz4),
            2 => Some(Compression::Zstd),
            _ => None,
        }
    }

    /// Retrouve une méthode depuis son nom en ligne de commande.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" | "aucune" => Some(Compression::None),
            "lz4" => Some(Compression::Lz4),
            "zstd" => Some(Compression::Zstd),
            "auto" => Some(Compression::Auto),
            _ => None,
        }
    }
}

/// Description d'une enveloppe, obtenue sans décompresser.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct EnvelopeInfo {
    pub original_len: u64,
    pub stored_len: u64,
    pub compression: Compression,
    pub sha256: [u8; 32],
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().expect("tranche de 8 octets"))
}

fn zstd_compress(data: &[u8]) -> Result<Vec<u8>> {
    zstd::encode_all(data, ZSTD_LEVEL).map_err(|e| DnaError::Compression(format!("zstd: {e}")))
}

/// Construit l'enveloppe d'un fichier.
pub fn pack(data: &[u8], compression: Compression) -> Result<Vec<u8>> {
    let digest = sha256(data);
    let (method, stored): (Compression, Vec<u8>) = match compression {
        Compression::None => (Compression::None, data.to_vec()),
        Compression::Lz4 => (
            Compression::Lz4,
            lz4_flex::block::compress_prepend_size(data),
        ),
        Compression::Zstd => (Compression::Zstd, zstd_compress(data)?),
        Compression::Auto => {
            let compressed = zstd_compress(data)?;
            if compressed.len() < data.len() {
                (Compression::Zstd, compressed)
            } else {
                (Compression::None, data.to_vec())
            }
        }
    };

    let mut body = Vec::with_capacity(HEADER_LEN + stored.len());
    body.extend_from_slice(&MAGIC);
    body.push(VERSION);
    body.push(method.code());
    body.extend_from_slice(&[0, 0]);
    body.extend_from_slice(&(data.len() as u64).to_le_bytes());
    body.extend_from_slice(&(stored.len() as u64).to_le_bytes());
    body.extend_from_slice(&digest);
    let crc = crc32_parts(&[&body, &stored]);
    body.extend_from_slice(&crc.to_le_bytes());
    body.extend_from_slice(&stored);
    Ok(body)
}

/// Méthode de compression d'une enveloppe déjà construite.
pub fn compression_of(body: &[u8]) -> Result<Compression> {
    inspect(body).map(|info| info.compression)
}

/// Lit l'en-tête et vérifie la structure (magic, version, longueurs, CRC), sans décompresser.
pub fn inspect(body: &[u8]) -> Result<EnvelopeInfo> {
    if body.len() < HEADER_LEN {
        return Err(DnaError::Integrity(format!(
            "enveloppe de {} octets, en-tête absent",
            body.len()
        )));
    }
    let header = &body[..HEADER_LEN];
    if header[..4] != MAGIC {
        return Err(DnaError::UnsupportedFormat(
            "magic absent : ces données ne sont pas une archive ADN2".to_string(),
        ));
    }
    if header[4] != VERSION {
        return Err(DnaError::UnsupportedFormat(format!(
            "version de format {} non prise en charge (attendue: {VERSION})",
            header[4]
        )));
    }
    let compression = Compression::from_code(header[5]).ok_or_else(|| {
        DnaError::UnsupportedFormat(format!("méthode de compression inconnue: {}", header[5]))
    })?;
    let original_len = read_u64(&header[8..16]);
    let stored_len = read_u64(&header[16..24]);
    let stored = &body[HEADER_LEN..];
    if stored_len != stored.len() as u64 {
        return Err(DnaError::Integrity(format!(
            "longueur des données stockées incohérente: {} annoncés, {} présents",
            stored_len,
            stored.len()
        )));
    }
    let expected_crc = u32::from_le_bytes(header[56..60].try_into().expect("tranche de 4 octets"));
    if crc32_parts(&[&header[..56], stored]) != expected_crc {
        return Err(DnaError::Integrity(
            "CRC-32 de l'enveloppe incorrect".to_string(),
        ));
    }
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&header[24..56]);
    Ok(EnvelopeInfo {
        original_len,
        stored_len,
        compression,
        sha256,
    })
}

fn decompress(method: Compression, stored: &[u8], original_len: u64) -> Result<Vec<u8>> {
    match method {
        Compression::None => Ok(stored.to_vec()),
        Compression::Lz4 => {
            if stored.len() < 4 {
                return Err(DnaError::Integrity("bloc LZ4 tronqué".to_string()));
            }
            let announced = u64::from(u32::from_le_bytes(
                stored[..4].try_into().expect("tranche de 4 octets"),
            ));
            if announced != original_len {
                return Err(DnaError::Integrity(
                    "taille annoncée par LZ4 incohérente".to_string(),
                ));
            }
            // Un bloc LZ4 ne peut pas s'étendre au-delà de ~255 fois sa taille.
            let bound = (stored.len() as u64 - 4) * 255 + 16;
            if original_len > bound {
                return Err(DnaError::Integrity(
                    "taux d'extension LZ4 invraisemblable".to_string(),
                ));
            }
            let mut out = vec![0u8; original_len as usize];
            let written = lz4_flex::block::decompress_into(&stored[4..], &mut out)
                .map_err(|e| DnaError::Integrity(format!("bloc LZ4 illisible: {e}")))?;
            out.truncate(written);
            Ok(out)
        }
        Compression::Zstd => {
            let limit = original_len.saturating_add(1);
            let decoder = zstd::stream::read::Decoder::new(stored)
                .map_err(|e| DnaError::Integrity(format!("flux zstd illisible: {e}")))?;
            let mut out = Vec::new();
            decoder
                .take(limit)
                .read_to_end(&mut out)
                .map_err(|e| DnaError::Integrity(format!("flux zstd illisible: {e}")))?;
            Ok(out)
        }
        Compression::Auto => Err(DnaError::UnsupportedFormat(
            "méthode de compression « auto » inattendue dans une archive".to_string(),
        )),
    }
}

/// Vérifie l'enveloppe et retourne le fichier original.
pub fn unpack(body: &[u8]) -> Result<Vec<u8>> {
    let info = inspect(body)?;
    let mut data = decompress(info.compression, &body[HEADER_LEN..], info.original_len)?;
    if data.len() as u64 != info.original_len {
        return Err(DnaError::Integrity(format!(
            "longueur décompressée {} au lieu de {}",
            data.len(),
            info.original_len
        )));
    }
    data.shrink_to_fit();
    if sha256(&data) != info.sha256 {
        return Err(DnaError::Integrity(
            "l'empreinte SHA-256 du fichier reconstitué ne correspond pas à celle de l'archive"
                .to_string(),
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: usize) -> Vec<u8> {
        match kind {
            0 => Vec::new(),
            1 => b"Hello".to_vec(),
            2 => b"ADN proof: archive de donnees dans de l'ADN. ".repeat(500),
            _ => {
                let mut rng = crate::codec::rng::SeededRng::new(0xC0FFEE, 1);
                let mut out = vec![0u8; 70_000];
                rng.fill_bytes(&mut out);
                out
            }
        }
    }

    #[test]
    fn roundtrip_for_every_compression() {
        for kind in 0..4 {
            let data = sample(kind);
            for compression in [
                Compression::None,
                Compression::Lz4,
                Compression::Zstd,
                Compression::Auto,
            ] {
                let body = pack(&data, compression).unwrap();
                assert_eq!(unpack(&body).unwrap(), data, "kind={kind} {compression:?}");
                let info = inspect(&body).unwrap();
                assert_eq!(info.original_len, data.len() as u64);
                assert_eq!(info.sha256, sha256(&data));
                assert_ne!(info.compression, Compression::Auto);
            }
        }
    }

    #[test]
    fn auto_keeps_uncompressed_when_compression_is_useless() {
        let random = sample(3);
        let info = inspect(&pack(&random, Compression::Auto).unwrap()).unwrap();
        assert_eq!(info.compression, Compression::None);
        let text = sample(2);
        let info = inspect(&pack(&text, Compression::Auto).unwrap()).unwrap();
        assert_eq!(info.compression, Compression::Zstd);
        assert!(info.stored_len < text.len() as u64 / 2);
    }

    #[test]
    fn any_single_bit_flip_is_detected() {
        let data = b"donnees a proteger, elles doivent etre rendues intactes ou pas du tout.";
        for compression in [Compression::None, Compression::Lz4, Compression::Zstd] {
            let body = pack(data, compression).unwrap();
            for position in 0..body.len() {
                for bit in [0u8, 3] {
                    let mut corrupted = body.clone();
                    corrupted[position] ^= 1 << bit;
                    let outcome = unpack(&corrupted);
                    let err =
                        outcome.expect_err("une enveloppe altérée ne doit jamais être rendue");
                    assert!(
                        matches!(err, DnaError::Integrity(_) | DnaError::UnsupportedFormat(_)),
                        "{err}"
                    );
                }
            }
        }
    }

    #[test]
    fn truncation_and_padding_are_detected() {
        let body = pack(&sample(2), Compression::Zstd).unwrap();
        for cut in [0, 1, HEADER_LEN - 1, HEADER_LEN, body.len() - 1] {
            assert!(unpack(&body[..cut]).is_err(), "cut={cut}");
        }
        let mut padded = body.clone();
        padded.push(0);
        assert!(unpack(&padded).is_err());
    }

    #[test]
    fn foreign_or_old_input_is_rejected_with_clear_errors() {
        assert!(matches!(
            unpack(b"pas une archive"),
            Err(DnaError::Integrity(_))
        ));
        let mut old = pack(b"x", Compression::None).unwrap();
        old[4] = 1;
        assert!(matches!(unpack(&old), Err(DnaError::UnsupportedFormat(_))));
        old[4] = VERSION;
        old[5] = 9;
        assert!(matches!(unpack(&old), Err(DnaError::UnsupportedFormat(_))));
    }

    #[test]
    fn compression_names() {
        for name in ["none", "ZSTD", "lz4", "auto", "aucune"] {
            assert!(Compression::from_name(name).is_some());
        }
        assert!(Compression::from_name("bzip2").is_none());
        assert_eq!(Compression::from_code(255), None);
    }
}
