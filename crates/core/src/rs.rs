//! Reed-Solomon systématique sur GF(256).
//!
//! Un bloc codé s'écrit `données || parité`, avec `ecc` octets de parité. Il corrige jusqu'à
//! `ecc / 2` octets erronés, ou jusqu'à `ecc` effacements dont les positions sont connues.
//!
//! L'encodage et le contrôle de parité sont réimplémentés ici sans allocation (c'est le chemin
//! parcouru par chaque oligo ou lecture) et produisent exactement les mêmes blocs que le crate
//! `reed-solomon`, dont le décodeur est utilisé pour la correction.

use crate::error::{DnaError, Result};
use reed_solomon::Decoder;

/// Corps GF(2^8) construit sur le polynôme primitif x^8 + x^4 + x^3 + x^2 + 1 (0x11D),
/// même corps que le crate `reed-solomon`.
mod gf {
    const fn build_tables() -> ([u8; 512], [u8; 256]) {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        let mut i = 0;
        while i < 255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11D;
            }
            i += 1;
        }
        while i < 512 {
            exp[i] = exp[i - 255];
            i += 1;
        }
        (exp, log)
    }

    static TABLES: ([u8; 512], [u8; 256]) = build_tables();

    #[inline]
    pub(super) fn exp(i: usize) -> u8 {
        TABLES.0[i]
    }

    #[inline]
    pub(super) fn log(x: u8) -> usize {
        TABLES.1[x as usize] as usize
    }

    /// Produit dans GF(256).
    #[inline]
    pub fn mul(a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            exp(log(a) + log(b))
        }
    }
}

/// Longueur maximale d'un bloc (le code est défini sur des positions d'un octet).
pub const MAX_BLOCK_LEN: usize = 255;

/// Code de Reed-Solomon pour une longueur de parité donnée.
#[derive(Clone, Debug)]
pub struct RsCode {
    ecc: usize,
    /// Polynôme générateur, degré le plus élevé en tête, coefficient dominant 1.
    generator: Vec<u8>,
}

impl RsCode {
    /// Crée un code avec `ecc` octets de parité (1 à 254).
    pub fn new(ecc: usize) -> Result<Self> {
        if ecc == 0 || ecc >= MAX_BLOCK_LEN {
            return Err(DnaError::InvalidConfig(format!(
                "parité Reed-Solomon invalide: {} octet(s)",
                ecc
            )));
        }
        // Racines α^0 … α^(ecc-1), comme le crate de référence.
        let mut generator = vec![1u8];
        for i in 0..ecc {
            let root = gf::exp(i);
            let mut next = vec![0u8; generator.len() + 1];
            next[0] = generator[0];
            for j in 1..generator.len() {
                next[j] = generator[j] ^ gf::mul(generator[j - 1], root);
            }
            next[generator.len()] = gf::mul(generator[generator.len() - 1], root);
            generator = next;
        }
        Ok(Self { ecc, generator })
    }

    /// Nombre d'octets de parité.
    pub fn ecc(&self) -> usize {
        self.ecc
    }

    /// Nombre maximal d'octets erronés corrigibles dans un bloc.
    pub fn correctable_errors(&self) -> usize {
        self.ecc / 2
    }

    /// Longueur maximale des données.
    pub fn max_data_len(&self) -> usize {
        MAX_BLOCK_LEN - self.ecc
    }

    /// Écrit la parité de `data` dans `parity` (de longueur `ecc`).
    ///
    /// La parité est le reste de la division de `data · x^ecc` par le polynôme générateur.
    pub fn parity(&self, data: &[u8], parity: &mut [u8]) -> Result<()> {
        if data.len() > self.max_data_len() || parity.len() != self.ecc {
            return Err(DnaError::Correction(format!(
                "bloc de {} octets de données incompatible avec une parité de {} octets",
                data.len(),
                self.ecc
            )));
        }
        parity.fill(0);
        for &symbol in data {
            let feedback = symbol ^ parity[0];
            parity.copy_within(1..self.ecc, 0);
            parity[self.ecc - 1] = 0;
            if feedback != 0 {
                let position = gf::log(feedback);
                for j in 1..=self.ecc {
                    parity[j - 1] ^= gf::exp(position + gf::log(self.generator[j]));
                }
            }
        }
        Ok(())
    }

    /// Retourne `données || parité`.
    pub fn encode(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut block = Vec::with_capacity(data.len() + self.ecc);
        block.extend_from_slice(data);
        block.resize(data.len() + self.ecc, 0);
        let (head, tail) = block.split_at_mut(data.len());
        self.parity(head, tail)?;
        Ok(block)
    }

    /// Vérifie la parité d'un bloc sans le modifier.
    pub fn check(&self, block: &[u8]) -> bool {
        if block.len() <= self.ecc || block.len() > MAX_BLOCK_LEN {
            return false;
        }
        let (data, parity) = block.split_at(block.len() - self.ecc);
        let mut computed = [0u8; MAX_BLOCK_LEN];
        let computed = &mut computed[..self.ecc];
        // Les longueurs ont déjà été validées : la parité ne peut pas échouer ici.
        self.parity(data, computed).is_ok() && computed == parity
    }

    /// Corrige `block` en place et retourne le nombre d'octets corrigés.
    ///
    /// Un bloc dont la parité est déjà valide rend 0. Un bloc irrécupérable rend une erreur.
    pub fn correct(&self, block: &mut [u8]) -> Result<usize> {
        if self.check(block) {
            return Ok(0);
        }
        let decoder = Decoder::new(self.ecc);
        let recovered = decoder
            .correct(block, None)
            .map_err(|_| DnaError::Correction("bloc Reed-Solomon irrécupérable".to_string()))?;
        let mut fixed = 0;
        for (slot, &value) in block.iter_mut().zip(recovered.iter()) {
            if *slot != value {
                *slot = value;
                fixed += 1;
            }
        }
        Ok(fixed)
    }

    /// Corrige `block` en place en s'appuyant sur des effacements de positions connues.
    ///
    /// Les octets effacés peuvent avoir une valeur arbitraire ; ils sont rétablis.
    pub fn correct_with_erasures(&self, block: &mut [u8], erasures: &[usize]) -> Result<()> {
        if erasures.len() > self.ecc || block.len() > MAX_BLOCK_LEN {
            return Err(DnaError::Correction(format!(
                "{} effacements pour {} octets de parité",
                erasures.len(),
                self.ecc
            )));
        }
        let mut positions: Vec<u8> = Vec::with_capacity(erasures.len());
        for &position in erasures {
            let Ok(index) = u8::try_from(position) else {
                return Err(DnaError::Correction(format!(
                    "position d'effacement hors bloc: {}",
                    position
                )));
            };
            if index as usize >= block.len() {
                return Err(DnaError::Correction(format!(
                    "position d'effacement {} hors bloc de {} octets",
                    position,
                    block.len()
                )));
            }
            block[position] = 0;
            positions.push(index);
        }
        let decoder = Decoder::new(self.ecc);
        let recovered = decoder
            .correct(block, Some(&positions))
            .map_err(|_| DnaError::Correction("bloc Reed-Solomon irrécupérable".to_string()))?;
        let data_len = block.len() - self.ecc;
        block[..data_len].copy_from_slice(&recovered[..data_len]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reed_solomon::Encoder;

    fn pseudo_random(len: usize, salt: u8) -> Vec<u8> {
        (0..len)
            .map(|i| ((i * 181 + usize::from(salt) * 29) ^ (i >> 3)) as u8)
            .collect()
    }

    #[test]
    fn parity_matches_reference_crate() {
        for ecc in [1, 2, 3, 4, 8, 16, 32] {
            let mine = RsCode::new(ecc).unwrap();
            let reference = Encoder::new(ecc);
            for len in [1, 7, 31, 36, 37, 100, 223, 255 - ecc] {
                let data = pseudo_random(len, ecc as u8);
                let expected = reference.encode(&data);
                let got = mine.encode(&data).unwrap();
                assert_eq!(&got[..len], &expected[..len], "ecc={ecc} len={len}");
                assert_eq!(&got[len..], expected.ecc(), "ecc={ecc} len={len}");
                assert!(mine.check(&got));
            }
        }
    }

    #[test]
    fn reference_vector_from_crate_tests() {
        let data: Vec<u8> = (0..30).collect();
        let block = RsCode::new(8).unwrap().encode(&data).unwrap();
        assert_eq!(&block[30..], &[99, 26, 219, 193, 9, 94, 186, 143]);
    }

    #[test]
    fn corrects_up_to_half_the_parity() {
        let mine = RsCode::new(6).unwrap();
        let data = pseudo_random(40, 1);
        let mut block = mine.encode(&data).unwrap();
        for i in [0, 7, 19] {
            block[i] ^= 0x5A;
        }
        assert_eq!(mine.correct(&mut block).unwrap(), 3);
        assert_eq!(&block[..40], &data[..]);
    }

    #[test]
    fn more_errors_than_correctable_never_yields_a_silent_bad_block() {
        // Avec 5 erreurs pour une capacité de 3, deux issues sont acceptables : un échec
        // annoncé, ou une correction vers un autre mot de code valide. Dans le second cas le
        // résultat est faux mais reste un mot de code : c'est précisément ce que l'empreinte
        // SHA-256 de l'enveloppe intercepte ensuite.
        let mine = RsCode::new(6).unwrap();
        let data = pseudo_random(40, 2);
        let mut block = mine.encode(&data).unwrap();
        for slot in block.iter_mut().take(10).step_by(2) {
            *slot ^= 0xFF;
        }
        assert!(!mine.check(&block));
        if mine.correct(&mut block).is_ok() {
            assert!(mine.check(&block));
        }
    }

    /// Trois substitutions bien placées forment un autre mot de code : la parité ne peut rien
    /// y voir. Avec deux octets de parité, la distance minimale vaut 3. C'est pourquoi DNA
    /// Fountain fait voter les lectures reçues sans erreur d'une même goutte, au lieu de croire
    /// la première venue.
    #[test]
    fn three_well_placed_errors_form_another_codeword() {
        let code = RsCode::new(2).unwrap();
        let data = pseudo_random(36, 5);
        let mut block = code.encode(&data).unwrap();
        for (position, delta) in [(14usize, 1u8), (17, 3), (35, 2)] {
            block[position] ^= delta;
        }
        assert!(code.check(&block), "la parité se vérifie pourtant");
        assert_eq!(
            code.correct(&mut block).unwrap(),
            0,
            "et rien n'est corrigé"
        );
        assert_ne!(
            &block[..36],
            &data[..],
            "alors que trois données sont fausses"
        );

        // Deux erreurs, en revanche, ne trompent jamais la parité.
        let mut block = code.encode(&data).unwrap();
        for (position, delta) in [(14usize, 1u8), (17, 3)] {
            block[position] ^= delta;
        }
        assert!(!code.check(&block));
    }

    #[test]
    fn erasures_are_recovered() {
        let mine = RsCode::new(32).unwrap();
        let data = pseudo_random(223, 3);
        let mut block = mine.encode(&data).unwrap();
        let erasures = [0usize, 5, 61, 200, 222, 223, 240, 254];
        for &position in &erasures {
            block[position] = 0xAB;
        }
        mine.correct_with_erasures(&mut block, &erasures).unwrap();
        assert_eq!(&block[..223], &data[..]);
    }

    #[test]
    fn rejects_inconsistent_parameters() {
        assert!(RsCode::new(0).is_err());
        assert!(RsCode::new(255).is_err());
        let code = RsCode::new(32).unwrap();
        let mut parity = [0u8; 32];
        assert!(code.parity(&vec![0u8; 224], &mut parity).is_err());
        assert!(!code.check(&[0u8; 32]));
    }
}
