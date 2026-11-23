//! Séquences ADN : représentation, validation et conversion.
//!
//! Une séquence est une suite d'octets ASCII `A`, `C`, `G`, `T`. Ce choix (un octet par
//! nucléotide) privilégie la simplicité des entrées/sorties FASTA et des contrôles de contraintes ;
//! les codecs ne manipulent les données utiles qu'après conversion en octets.

use crate::error::{DnaError, Result};

/// Les quatre bases, dans l'ordre des valeurs 2 bits 0, 1, 2, 3.
pub const BASES: [u8; 4] = *b"ACGT";

/// Longueur de ligne utilisée pour écrire les fichiers FASTA.
pub const FASTA_LINE_WIDTH: usize = 60;

#[inline]
pub fn is_gc(base: u8) -> bool {
    base == b'G' || base == b'C'
}

#[inline]
pub fn base_to_bits(base: u8) -> Option<u8> {
    match base {
        b'A' => Some(0),
        b'C' => Some(1),
        b'G' => Some(2),
        b'T' => Some(3),
        _ => None,
    }
}

/// Nombre de bases G et C.
pub fn gc_count(seq: &[u8]) -> usize {
    seq.iter().filter(|&&b| is_gc(b)).count()
}

/// Part de G et C (0 si la séquence est vide).
pub fn gc_content(seq: &[u8]) -> f64 {
    if seq.is_empty() {
        0.0
    } else {
        gc_count(seq) as f64 / seq.len() as f64
    }
}

/// Longueur de la plus longue suite de bases identiques.
pub fn max_homopolymer(seq: &[u8]) -> usize {
    let mut best = 0;
    let mut run = 0;
    let mut previous = 0u8;
    for &base in seq {
        if base == previous {
            run += 1;
        } else {
            run = 1;
            previous = base;
        }
        best = best.max(run);
    }
    best
}

/// Vérifie que tous les caractères sont des bases standard (casse indifférente).
pub fn is_dna(seq: &[u8]) -> bool {
    seq.iter()
        .all(|&b| matches!(b, b'A' | b'C' | b'G' | b'T' | b'a' | b'c' | b'g' | b't'))
}

/// Met en majuscules et retire les blancs ; refuse tout autre caractère.
pub fn normalize(raw: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len());
    for &b in raw {
        match b {
            b'A' | b'C' | b'G' | b'T' => out.push(b),
            b'a' | b'c' | b'g' | b't' => out.push(b.to_ascii_uppercase()),
            b' ' | b'\t' | b'\r' | b'\n' => {}
            _ => {
                return Err(DnaError::InvalidDna(format!(
                    "caractère {:?} inattendu (attendu : A, C, G ou T)",
                    b as char
                )));
            }
        }
    }
    Ok(out)
}

/// Complément d'une base standard (majuscule).
#[inline]
pub fn complement(base: u8) -> u8 {
    match base {
        b'A' => b'T',
        b'C' => b'G',
        b'G' => b'C',
        _ => b'A',
    }
}

/// Complément inversé : la séquence lue de l'autre bout, sur le brin complémentaire.
pub fn reverse_complement(seq: &[u8]) -> Vec<u8> {
    seq.iter().rev().map(|&base| complement(base)).collect()
}

/// Complément inversé, en place.
pub fn reverse_complement_in_place(seq: &mut [u8]) {
    seq.reverse();
    for base in seq.iter_mut() {
        *base = complement(*base);
    }
}

/// Convertit des octets en ADN : 2 bits par nucléotide, poids fort en tête.
pub fn bytes_to_dna(bytes: &[u8], out: &mut Vec<u8>) {
    out.reserve(bytes.len() * 4);
    for &byte in bytes {
        out.push(BASES[(byte >> 6) as usize]);
        out.push(BASES[((byte >> 4) & 3) as usize]);
        out.push(BASES[((byte >> 2) & 3) as usize]);
        out.push(BASES[(byte & 3) as usize]);
    }
}

/// Convertit un ADN en octets. Erreur si la longueur n'est pas multiple de 4 ou si un caractère
/// n'est pas une base standard.
pub fn dna_to_bytes(dna: &[u8]) -> Result<Vec<u8>> {
    if !dna.len().is_multiple_of(4) {
        return Err(DnaError::InvalidDna(format!(
            "longueur {} non multiple de 4",
            dna.len()
        )));
    }
    let mut out = Vec::with_capacity(dna.len() / 4);
    // La longueur est multiple de 4 : il n'y a pas de reste.
    for quad in dna.as_chunks::<4>().0 {
        let mut byte = 0u8;
        for (i, &base) in quad.iter().enumerate() {
            let bits = base_to_bits(base).ok_or_else(|| {
                DnaError::InvalidDna(format!(
                    "caractère {:?} inattendu (attendu : A, C, G ou T)",
                    base as char
                ))
            })?;
            byte |= bits << (6 - 2 * i);
        }
        out.push(byte);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dna_roundtrip_all_bytes() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let mut dna = Vec::new();
        bytes_to_dna(&bytes, &mut dna);
        assert_eq!(dna.len(), bytes.len() * 4);
        assert_eq!(dna_to_bytes(&dna).unwrap(), bytes);
    }

    #[test]
    fn dna_mapping_is_two_bits_msb_first() {
        let mut dna = Vec::new();
        bytes_to_dna(&[0b0001_1011, 0xFF, 0x00], &mut dna);
        assert_eq!(dna, b"ACGTTTTTAAAA");
    }

    #[test]
    fn dna_to_bytes_rejects_bad_input() {
        assert!(dna_to_bytes(b"ACG").is_err());
        assert!(dna_to_bytes(b"ACGN").is_err());
        assert!(dna_to_bytes(b"acgt").is_err());
    }

    #[test]
    fn homopolymer_and_gc() {
        assert_eq!(max_homopolymer(b""), 0);
        assert_eq!(max_homopolymer(b"AAACGTAAAAG"), 4);
        assert_eq!(gc_count(b"ACGT"), 2);
        assert_eq!(gc_content(b""), 0.0);
        assert!((gc_content(b"GGCC") - 1.0).abs() < 1e-12);
    }

    #[test]
    fn reverse_complement_reads_the_other_strand() {
        assert_eq!(reverse_complement(b"ACGTTGCA"), b"TGCAACGT");
        assert_eq!(reverse_complement(b""), b"");
        assert_eq!(
            reverse_complement(&reverse_complement(b"GGATTACA")),
            b"GGATTACA"
        );
        let mut seq = b"ACGT".to_vec();
        reverse_complement_in_place(&mut seq);
        assert_eq!(seq, b"ACGT");
        let mut seq = b"AAG".to_vec();
        reverse_complement_in_place(&mut seq);
        assert_eq!(seq, b"CTT");
    }

    #[test]
    fn normalize_tolerates_case_and_space_only() {
        assert_eq!(normalize(b"ac gt\nAC").unwrap(), b"ACGTAC");
        assert!(normalize(b"ACGN").is_err());
        assert!(normalize(b"ACG1").is_err());
        assert!(is_dna(b"acgt"));
        assert!(!is_dna(b"acgn"));
    }
}
