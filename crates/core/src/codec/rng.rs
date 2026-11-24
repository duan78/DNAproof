//! Générateur pseudo-aléatoire déterministe.
//!
//! Le décodeur recalcule la structure des gouttes en rejouant les tirages de l'encodeur.
//! L'algorithme fait donc partie du format (voir `docs/FORMAT.md`) : il est figé et ne doit
//! jamais changer, ce qui protège aussi les archives d'une mise à jour du générateur d'une
//! bibliothèque externe.
//!
//! Il s'agit de SplitMix64, complété par deux échantillonneurs : un réel uniforme dans `[0, 1)`
//! et un entier uniforme dans `[0, n)` par la méthode de Lemire.

/// Constante de séparation des flux : deux usages distincts du générateur (degré d'une goutte,
/// sélection des blocs) ne partagent jamais le même flux.
pub const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Clone, Debug)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// Ouvre le flux numéro `stream` pour la graine `seed`.
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Self { state: 0 };
        // Mélange (graine, flux) pour que deux flux diffèrent dès le premier tirage.
        rng.state = rng.mix(seed ^ stream.wrapping_mul(GOLDEN_GAMMA));
        rng
    }

    fn mix(&self, mut z: u64) -> u64 {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Entier de 64 bits.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        self.mix(self.state)
    }

    /// Entier de 32 bits.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Remplit `out` d'octets uniformes.
    pub fn fill_bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let value = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&value[..chunk.len()]);
        }
    }

    /// Réel uniforme dans `[0, 1)`.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Entier uniforme dans `[0, n)` ; `n` doit être non nul.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        loop {
            let product = u128::from(self.next_u64()) * u128::from(n);
            let low = product as u64;
            if low >= n {
                return (product >> 64) as u64;
            }
            let threshold = n.wrapping_neg() % n;
            if low >= threshold {
                return (product >> 64) as u64;
            }
        }
    }
}

/// Bijection de 32 bits qui disperse un compteur.
///
/// Les graines des gouttes sont des numéros de suite, mais un compteur brut s'écrirait en ADN
/// comme de longues suites de A (rejetées par le criblage). La bijection donne des graines
/// uniques d'apparence aléatoire, comme le registre à décalage de l'article.
pub fn scramble_u32(value: u32) -> u32 {
    let mut x = value;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

/// Bijection de 24 bits qui disperse un compteur (même rôle que [`scramble_u32`]).
pub fn scramble_u24(value: u32) -> u32 {
    const MASK: u32 = (1 << 24) - 1;
    // Chaque étape est une bijection sur 24 bits : décalage exclusif, ou exclusif avec une
    // constante, multiplication par un impair modulo 2^24. La constante évite que zéro se
    // représente lui-même.
    let mut x = (value ^ (value >> 11) ^ 0x5A_3C_7E) & MASK;
    x = x.wrapping_mul(0x80_00_15) & MASK;
    x ^= x >> 13;
    x = x.wrapping_mul(0x6F_4F_2D) & MASK;
    x ^ (x >> 12)
}

/// L'inverse d'un nombre impair modulo 2⁶⁴, donc modulo 2²⁴ : itération de Newton.
pub(crate) const fn inverse_odd(odd: u64) -> u64 {
    let mut inverse = 1u64;
    let mut round = 0;
    while round < 6 {
        inverse = inverse.wrapping_mul(2u64.wrapping_sub(odd.wrapping_mul(inverse)));
        round += 1;
    }
    inverse
}

/// L'inverse de [`scramble_u24`].
pub fn unscramble_u24(value: u32) -> u32 {
    const MASK: u32 = (1 << 24) - 1;
    let mut x = value & MASK;
    // Chaque étape s'inverse : un ou exclusif avec un décalage de `shift` d'abord, la constante
    // ensuite, une multiplication par son inverse.
    x ^= x >> 12;
    x = x.wrapping_mul(inverse_odd(0x6F_4F_2D) as u32) & MASK;
    x ^= x >> 13;
    x = x.wrapping_mul(inverse_odd(0x80_00_15) as u32) & MASK;
    x ^= 0x5A_3C_7E;
    x ^= x >> 11;
    x ^= x >> 22;
    x & MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_stream_separated() {
        let mut a = SeededRng::new(42, 1);
        let mut b = SeededRng::new(42, 1);
        let mut c = SeededRng::new(42, 2);
        let mut d = SeededRng::new(43, 1);
        for _ in 0..16 {
            let x = a.next_u64();
            assert_eq!(x, b.next_u64());
            assert_ne!(x, c.next_u64());
            assert_ne!(x, d.next_u64());
        }
    }

    #[test]
    fn frozen_vectors() {
        // Valeurs de référence : tout changement casse la lecture des archives existantes.
        let mut rng = SeededRng::new(0, 0);
        let values: Vec<u64> = (0..4).map(|_| rng.next_u64()).collect();
        assert_eq!(
            values,
            [
                0xe220_a839_7b1d_cdaf,
                0x6e78_9e6a_a1b9_65f4,
                0x06c4_5d18_8009_454f,
                0xf88b_b8a8_724c_81ec,
            ]
        );
        let mut rng = SeededRng::new(0, 0);
        rng.next_u64();
        assert_eq!(rng.next_f64().to_bits(), 0x3fdb_9e27_9aa8_6e58);
        let mut rng = SeededRng::new(0, 0);
        assert_eq!(
            [
                rng.below(10),
                rng.below(1000),
                rng.below(7),
                rng.below(1 << 40),
            ],
            [8, 431, 0, 1_067_496_024_178]
        );
        // Les quatre flux d'une même graine divergent dès le premier tirage.
        let first: Vec<u64> = (0..8u64)
            .map(|stream| SeededRng::new(1234, stream).next_u64())
            .collect();
        let mut distinct = first.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), first.len());
    }

    #[test]
    fn below_is_uniform_on_a_power_of_two() {
        let mut rng = SeededRng::new(7, 3);
        let n = 8u64;
        let mut counts = [0u64; 8];
        for _ in 0..80_000 {
            counts[rng.below(n) as usize] += 1;
        }
        for count in counts {
            assert!((9_500..=10_500).contains(&count), "{counts:?}");
        }
        for n in [1u64, 2, 3, 5, 255, 4096, 1 << 40] {
            let value = rng.below(n);
            assert!(value < n);
        }
    }

    #[test]
    fn next_f64_stays_in_unit_interval() {
        let mut rng = SeededRng::new(1, 1);
        for _ in 0..10_000 {
            let x = rng.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn fill_bytes_handles_partial_tail() {
        let mut rng = SeededRng::new(5, 9);
        let mut out = [0u8; 13];
        rng.fill_bytes(&mut out);
        let mut again = SeededRng::new(5, 9);
        let mut expected = [0u8; 16];
        again.fill_bytes(&mut expected);
        assert_eq!(out, expected[..13]);
    }

    #[test]
    fn scramble_u24_is_a_bijection() {
        let mut seen = vec![false; 1 << 24];
        for i in 0..(1u32 << 24) {
            let mapped = scramble_u24(i) as usize;
            assert!(mapped < (1 << 24));
            // Et l'inverse la défait.
            assert_eq!(unscramble_u24(mapped as u32), i);
            assert!(
                !std::mem::replace(&mut seen[mapped], true),
                "collision pour {i}"
            );
        }
        // Zéro ne se représente pas lui-même : pas de suite de A en tête d'oligo.
        assert_ne!(scramble_u24(0), 0);
        assert_ne!(scramble_u24(0) & 0xFF_0000, 0);
        assert_ne!(scramble_u24(0) & 0x00_00FF, 0);
    }

    #[test]
    fn scramble_is_a_bijection() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..200_000u32 {
            assert!(seen.insert(scramble_u32(i)));
        }
        assert_ne!(scramble_u32(1), 1);
    }
}
