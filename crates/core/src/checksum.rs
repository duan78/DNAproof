//! Empreintes et sommes de contrôle.
//!
//! - SHA-256 garantit l'intégrité de bout en bout : l'empreinte du fichier original voyage dans
//!   l'enveloppe et est comparée au résultat avant de le rendre.
//! - CRC-32 sert de contrôle structurel rapide (en-tête d'enveloppe, manifeste).

use sha2::{Digest, Sha256};

/// Empreinte SHA-256 de `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// SHA-256 de plusieurs tranches concaténées, sans les recopier.
pub fn sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC32_TABLE: [u32; 256] = crc32_table();

/// Poursuit un CRC-32 (IEEE 802.3) avec l'état interne donné.
fn crc32_feed(mut state: u32, data: &[u8]) -> u32 {
    for &byte in data {
        state = CRC32_TABLE[((state ^ u32::from(byte)) & 0xFF) as usize] ^ (state >> 8);
    }
    state
}

/// CRC-32 (IEEE 802.3, polynôme réfléchi 0xEDB88320) de `data`.
pub fn crc32(data: &[u8]) -> u32 {
    crc32_feed(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}

/// CRC-32 de plusieurs tranches concaténées.
pub fn crc32_parts(parts: &[&[u8]]) -> u32 {
    let mut state = 0xFFFF_FFFF;
    for part in parts {
        state = crc32_feed(state, part);
    }
    state ^ 0xFFFF_FFFF
}

/// Représentation hexadécimale minuscule.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0F) as usize] as char);
    }
    out
}

/// Lit une empreinte SHA-256 donnée en hexadécimal (64 chiffres, casse indifférente).
pub fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = text.as_bytes()[2 * i];
        let lo = text.as_bytes()[2 * i + 1];
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        };
        *slot = digit(hi)? << 4 | digit(lo)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_reference_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn crc32_parts_equals_single_pass() {
        let data = b"reunies en une seule passe ou en plusieurs";
        assert_eq!(
            crc32(data),
            crc32_parts(&[&data[..10], &data[10..25], &data[25..]])
        );
    }

    #[test]
    fn sha256_reference_vector() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256(b"abc"), sha256_parts(&[b"a", b"bc"]));
    }

    #[test]
    fn parse_sha256_accepts_both_cases_only() {
        let digest = sha256(b"x");
        assert_eq!(parse_sha256(&hex(&digest)), Some(digest));
        assert_eq!(parse_sha256(&hex(&digest).to_uppercase()), Some(digest));
        assert_eq!(parse_sha256("zz"), None);
        assert_eq!(parse_sha256(&hex(&digest)[..63]), None);
    }
}
