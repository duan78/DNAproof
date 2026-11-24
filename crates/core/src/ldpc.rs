//! Code LDPC systématique, de construction déterministe, décodé par propagation de croyances.
//!
//! Le mot de code compte `k` bits d'information suivis de `m` bits de parité, un par contrôle.
//! Chaque bit d'information participe à [`COLUMN_WEIGHT`] contrôles ; le bit de parité `j`
//! participe aux contrôles `j` et `j + 1`. L'encodage est une simple accumulation : la parité
//! du contrôle `j` vaut celle du contrôle `j − 1`, plus la somme des bits d'information du
//! contrôle `j`.
//!
//! La matrice de contrôle n'est rangée nulle part : elle se reconstruit à l'identique d'une
//! graine, chez l'encodeur comme chez le décodeur. Ses colonnes sont tirées au hasard en évitant
//! les cycles de longueur quatre — deux bits d'information qui participeraient aux mêmes deux
//! contrôles — et en répartissant la charge des contrôles.
//!
//! Le décodage est celui du *min-sum* normalisé : des va-et-vient entre variables et contrôles,
//! sur des rapports de vraisemblance `ln P(0) / P(1)`. Un rapport nul est un effacement. Le
//! décodage s'arrête dès que chaque contrôle est satisfait ; à défaut, il rend son estimation et
//! l'échec.

use crate::codec::rng::SeededRng;
use crate::error::{DnaError, Result};
use std::collections::HashSet;

/// Contrôles auxquels participe chaque bit d'information. À quatre, le code tient le mieux
/// les erreurs : mesuré sur un bloc de 102 400 bits au rendement 0,8, 2 % d'erreurs sont
/// corrigées, contre 1,5 % avec trois contrôles.
pub const COLUMN_WEIGHT: usize = 4;
/// Facteur de normalisation du min-sum : sa correction est trop optimiste, on la réduit.
pub const NORMALIZATION: f32 = 0.8;
/// Essais au-delà desquels un cycle de longueur quatre est admis, faute de mieux.
const ATTEMPTS: usize = 200;
const STREAM: u64 = 0x0000_0000_0000_0013;

/// Un code LDPC, défini par ses longueurs et sa graine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LdpcCode {
    k: usize,
    m: usize,
    column_weight: usize,
    /// Arêtes rangées par contrôle : la variable que chacune relie.
    edge_variable: Box<[u32]>,
    /// Le contrôle de chaque arête.
    edge_check: Box<[u32]>,
    /// Début des arêtes de chaque contrôle, et fin des dernières.
    check_start: Box<[u32]>,
    /// Arêtes rangées par variable : leur rang dans l'ordre des contrôles.
    edge_of_variable: Box<[u32]>,
    /// Début des arêtes de chaque variable, et fin des dernières.
    variable_start: Box<[u32]>,
}

/// Résultat d'un décodage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Bits estimés : information puis parité.
    pub bits: Vec<u8>,
    /// Itérations faites.
    pub iterations: usize,
    /// Vrai si chaque contrôle est satisfait.
    pub success: bool,
}

fn ordered(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

impl LdpcCode {
    /// Bits d'information du code.
    pub fn k(&self) -> usize {
        self.k
    }

    /// Bits de parité, un par contrôle.
    pub fn m(&self) -> usize {
        self.m
    }

    /// Longueur du mot de code.
    pub fn codeword_len(&self) -> usize {
        self.k + self.m
    }

    /// Contrôles auxquels participe chaque bit d'information.
    pub fn column_weight(&self) -> usize {
        self.column_weight
    }

    /// Le code de ces longueurs et de cette graine. Deux constructions de mêmes paramètres sont
    /// identiques.
    ///
    /// # Panics
    ///
    /// Sans bit d'information, sans bit de parité, ou sans contrôle.
    pub fn new(k: usize, m: usize, seed: u64) -> Self {
        Self::with_column_weight(k, m, COLUMN_WEIGHT, seed)
    }

    /// Comme [`LdpcCode::new`], avec `column_weight` contrôles par bit d'information. Un poids
    /// plus fort tient mieux les erreurs, et coûte davantage d'arêtes.
    ///
    /// # Panics
    ///
    /// Sans bit d'information, sans bit de parité, ou avec moins de contrôles que le poids
    /// demandé.
    pub fn with_column_weight(k: usize, m: usize, column_weight: usize, seed: u64) -> Self {
        let weight = column_weight;
        assert!(
            k > 0 && m > 0,
            "un code LDPC a des bits d'information et de parité"
        );
        assert!(
            weight > 0 && m >= weight,
            "il faut {weight} contrôles au moins, autant que par bit d'information"
        );
        let mut rng = SeededRng::new(seed, STREAM);
        let mean = (weight * k) as f64 / m as f64;
        let mut load = vec![0u32; m];
        let mut shared: HashSet<(u32, u32)> = HashSet::with_capacity(weight * k);
        // Chaque bit de parité relie deux contrôles voisins : un bit d'information qui irait aux
        // deux en ferait un cycle de quatre.
        shared.extend((0..m as u32 - 1).map(|check| (check, check + 1)));
        let mut checks_of = Vec::with_capacity(k);
        for _ in 0..k {
            let mut chosen: Vec<u32> = Vec::with_capacity(weight);
            let mut attempts = 0;
            while chosen.len() < weight {
                let candidate = rng.below(m as u64) as u32;
                attempts += 1;
                if chosen.contains(&candidate) {
                    continue;
                }
                // Un cycle de quatre : deux bits aux mêmes deux contrôles.
                let crowded = chosen
                    .iter()
                    .any(|&other| shared.contains(&ordered(other, candidate)));
                // Répartition : d'autant plus réticent que le contrôle est chargé.
                let reluctant =
                    rng.next_f64() * f64::from(load[candidate as usize] + 1) > mean + 1.0;
                if (crowded || reluctant) && attempts < ATTEMPTS {
                    continue;
                }
                chosen.push(candidate);
            }
            for first in 0..weight {
                for second in first + 1..weight {
                    shared.insert(ordered(chosen[first], chosen[second]));
                }
            }
            for &check in &chosen {
                load[check as usize] += 1;
            }
            checks_of.push(chosen);
        }

        // Arêtes par contrôle : ses bits d'information, puis ses bits de parité (deux, un seul
        // pour le premier contrôle).
        let mut check_start = vec![0u32; m + 1];
        for checks in &checks_of {
            for &check in checks {
                check_start[check as usize + 1] += 1;
            }
        }
        check_start[1] += 1;
        for start in check_start.iter_mut().skip(2) {
            *start += 2;
        }
        for check in 0..m {
            check_start[check + 1] += check_start[check];
        }
        let mut cursor: Vec<usize> = check_start[..m]
            .iter()
            .map(|&start| start as usize)
            .collect();
        let edges = check_start[m] as usize;
        let mut edge_variable = vec![0u32; edges];
        let mut edge_check = vec![0u32; edges];
        for (variable, checks) in checks_of.iter().enumerate() {
            for &check in checks {
                let slot = &mut cursor[check as usize];
                edge_variable[*slot] = variable as u32;
                edge_check[*slot] = check;
                *slot += 1;
            }
        }
        for (check, slot) in cursor.iter_mut().enumerate() {
            edge_variable[*slot] = (k + check) as u32;
            edge_check[*slot] = check as u32;
            *slot += 1;
            if check > 0 {
                edge_variable[*slot] = (k + check - 1) as u32;
                edge_check[*slot] = check as u32;
                *slot += 1;
            }
        }

        // Les mêmes arêtes, rangées par variable.
        let total = k + m;
        let mut variable_start = vec![0u32; total + 1];
        for &variable in &edge_variable {
            variable_start[variable as usize + 1] += 1;
        }
        for variable in 0..total {
            variable_start[variable + 1] += variable_start[variable];
        }
        let mut filled: Vec<usize> = variable_start[..total]
            .iter()
            .map(|&start| start as usize)
            .collect();
        let mut edge_of_variable = vec![0u32; edges];
        for (edge, &variable) in edge_variable.iter().enumerate() {
            edge_of_variable[filled[variable as usize]] = edge as u32;
            filled[variable as usize] += 1;
        }

        Self {
            k,
            m,
            column_weight,
            edge_variable: edge_variable.into_boxed_slice(),
            edge_check: edge_check.into_boxed_slice(),
            check_start: check_start.into_boxed_slice(),
            edge_of_variable: edge_of_variable.into_boxed_slice(),
            variable_start: variable_start.into_boxed_slice(),
        }
    }

    fn edges_of_variable(&self, variable: usize) -> &[u32] {
        &self.edge_of_variable
            [self.variable_start[variable] as usize..self.variable_start[variable + 1] as usize]
    }

    fn edges_of_check(&self, check: usize) -> &[u32] {
        &self.edge_variable[self.check_start[check] as usize..self.check_start[check + 1] as usize]
    }

    /// Contrôles auxquels participe la variable : bits d'information puis bits de parité.
    pub fn checks_of(&self, variable: usize) -> Vec<usize> {
        self.edges_of_variable(variable)
            .iter()
            .map(|&edge| self.edge_check[edge as usize] as usize)
            .collect()
    }

    /// Vrai si deux variables participent aux mêmes deux contrôles : un cycle de longueur
    /// quatre, qui nuirait à la propagation.
    pub fn has_four_cycle(&self) -> bool {
        let mut seen = HashSet::new();
        (0..self.codeword_len()).any(|variable| {
            let checks = self.checks_of(variable);
            checks.iter().enumerate().any(|(first, &a)| {
                checks[first + 1..]
                    .iter()
                    .any(|&b| !seen.insert(ordered(a as u32, b as u32)))
            })
        })
    }

    /// Mot de code des `k` bits d'information : information puis parité.
    pub fn encode(&self, info: &[u8]) -> Result<Vec<u8>> {
        if info.len() != self.k {
            return Err(DnaError::InvalidConfig(format!(
                "code LDPC de {} bits d'information, reçu {}",
                self.k,
                info.len()
            )));
        }
        // Accumulation : la parité d'un contrôle vaut la précédente, plus la somme des bits
        // d'information du contrôle.
        let mut codeword = vec![0u8; self.k + self.m];
        codeword[..self.k].copy_from_slice(info);
        let mut carried = 0u8;
        for check in 0..self.m {
            let mut sum = carried;
            for &variable in self.edges_of_check(check) {
                let variable = variable as usize;
                if variable < self.k {
                    sum ^= info[variable];
                }
            }
            codeword[self.k + check] = sum;
            carried = sum;
        }
        Ok(codeword)
    }

    /// Vrai si chaque contrôle est satisfait par ce mot de code.
    pub fn check(&self, codeword: &[u8]) -> bool {
        if codeword.len() != self.codeword_len() {
            return false;
        }
        (0..self.m).all(|check| {
            self.edges_of_check(check)
                .iter()
                .fold(0u8, |parity, &variable| {
                    parity ^ codeword[variable as usize]
                })
                == 0
        })
    }

    /// Estime le mot de code depuis les rapports de vraisemblance du canal, un par bit.
    ///
    /// La propagation s'arrête dès que chaque contrôle est satisfait, et au plus après
    /// `max_iterations` allers-retours (un au moins). Une estimation qui ne satisfait pas les
    /// contrôles est rendue avec `success` à faux.
    pub fn decode(&self, channel: &[f32], max_iterations: usize) -> Result<Decoded> {
        let total = self.codeword_len();
        if channel.len() != total {
            return Err(DnaError::InvalidConfig(format!(
                "code LDPC de {} bits, reçu {} rapports",
                total,
                channel.len()
            )));
        }
        let edges = self.edge_variable.len();
        // Messages des variables vers les contrôles, puis l'inverse.
        let mut to_check = vec![0f32; edges];
        let mut to_variable = vec![0f32; edges];
        for (edge, &variable) in self.edge_variable.iter().enumerate() {
            to_check[edge] = channel[variable as usize];
        }
        let mut bits = vec![0u8; total];
        let mut iterations = 0;
        loop {
            // 1. Contrôles vers variables : le message vers une variable est le plus faible des
            //    autres, du signe du produit des signes, réduit par la normalisation.
            for check in 0..self.m {
                let messages = &to_check
                    [self.check_start[check] as usize..self.check_start[check + 1] as usize];
                let (mut lowest, mut next, mut sign) = (f32::INFINITY, f32::INFINITY, 1f32);
                for &message in messages {
                    let magnitude = message.abs();
                    if magnitude < lowest {
                        next = lowest;
                        lowest = magnitude;
                    } else if magnitude < next {
                        next = magnitude;
                    }
                    if message < 0.0 {
                        sign = -sign;
                    }
                }
                for (offset, &message) in messages.iter().enumerate() {
                    let magnitude = if message.abs() == lowest {
                        next
                    } else {
                        lowest
                    };
                    to_variable[self.check_start[check] as usize + offset] =
                        sign * message.signum() * magnitude * NORMALIZATION;
                }
            }
            // 2. Variables vers contrôles, et décision à leur somme.
            for variable in 0..total {
                let edges = self.edges_of_variable(variable);
                let mut sum = channel[variable];
                for &edge in edges {
                    sum += to_variable[edge as usize];
                }
                bits[variable] = u8::from(sum < 0.0);
                for &edge in edges {
                    to_check[edge as usize] = sum - to_variable[edge as usize];
                }
            }
            iterations += 1;
            // 3. Contrôles.
            let satisfied = (0..self.m).all(|check| {
                self.edges_of_check(check)
                    .iter()
                    .fold(0u8, |parity, &variable| parity ^ bits[variable as usize])
                    == 0
            });
            if satisfied || iterations >= max_iterations.max(1) {
                return Ok(Decoded {
                    bits,
                    iterations,
                    success: satisfied,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_bits(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 0x41);
        (0..len).map(|_| rng.below(2) as u8).collect()
    }

    /// Rapports d'un mot de code lu avec une confiance de 4 : `flipped` bits sont accusés à
    /// tort, `erased` sont effacés (rapport nul).
    fn channel(codeword: &[u8], flipped: usize, erased: usize, seed: u64) -> Vec<f32> {
        let mut positions: Vec<usize> = (0..codeword.len()).collect();
        let mut rng = SeededRng::new(seed, 0x42);
        for index in (1..positions.len()).rev() {
            positions.swap(index, rng.below(index as u64 + 1) as usize);
        }
        let mut llrs: Vec<f32> = codeword.iter().map(|&bit| 4.0 - 8.0 * bit as f32).collect();
        for &position in &positions[..flipped] {
            llrs[position] = -llrs[position];
        }
        for &position in &positions[flipped..flipped + erased] {
            llrs[position] = 0.0;
        }
        llrs
    }

    #[test]
    fn the_construction_is_deterministic_balanced_and_without_short_cycles() {
        let code = LdpcCode::new(4000, 1000, 7);
        assert_eq!(code, LdpcCode::new(4000, 1000, 7));
        assert_ne!(code, LdpcCode::new(4000, 1000, 8));
        assert_eq!(code.k(), 4001 - 1);
        assert_eq!(
            (code.k(), code.m(), code.codeword_len()),
            (4000, 1000, 5000)
        );
        // Trois contrôles par bit d'information, deux par bit de parité (un au premier).
        assert_eq!(code.edge_variable.len(), 4 * 4000 + 2 * 1000 - 1);
        for variable in 0..4000 {
            let checks = code.checks_of(variable);
            assert_eq!(checks.len(), 4, "variable {variable}");
            let mut distinct = checks.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), 4);
        }
        // Le premier bit de parité n'a qu'un contrôle : le bit de parité j va aux contrôles j
        // et j + 1.
        assert_eq!(code.checks_of(4000), vec![0, 1]);
        assert_eq!(code.checks_of(4500), vec![500, 501]);
        assert_eq!(code.checks_of(4999), vec![999]);
        assert!(!code.has_four_cycle());
        // La charge des contrôles reste autour de la moyenne : quatre bits d'information par
        // contrôle de parité, plus leurs bits de parité.
        // Le premier contrôle ne compte qu'un bit de parité.
        let info_loads: Vec<usize> = (0..1000)
            .map(|check| code.edges_of_check(check).len() - 2 + usize::from(check == 0))
            .collect();
        assert_eq!(info_loads.iter().sum::<usize>(), 4 * 4000);
        assert!(
            info_loads.iter().all(|&load| (6..=30).contains(&load)),
            "{info_loads:?}"
        );
    }

    #[test]
    fn every_codeword_satisfies_the_checks() {
        let code = LdpcCode::new(4000, 1000, 11);
        assert!(code.check(&code.encode(&[0; 4000]).unwrap()));
        for seed in 0..20u64 {
            let codeword = code.encode(&random_bits(4000, seed)).unwrap();
            assert!(code.check(&codeword));
            // Un bit retourné est toujours vu : chaque variable a au moins un contrôle.
            for position in [0, 1999, 4000, 4999] {
                let mut damaged = codeword.clone();
                damaged[position] ^= 1;
                assert!(!code.check(&damaged));
            }
        }
        assert!(code.encode(&[0; 3999]).is_err());
        assert!(!code.check(&[0; 4999]));
    }

    #[test]
    fn propagation_of_beliefs_corrects_errors_and_erasures() {
        let code = LdpcCode::new(16_384, 4096, 5);
        for seed in 0..4u64 {
            let codeword = code.encode(&random_bits(16_384, seed)).unwrap();
            let clean = code.decode(&channel(&codeword, 0, 0, seed), 100).unwrap();
            assert!(clean.success);
            assert_eq!(clean.bits, codeword);
            assert!(clean.iterations <= 2, "{}", clean.iterations);
            // Des erreurs, des effacements, et les deux : corrigés.
            for (flipped, erased) in [(0, 2000), (250, 0), (150, 1200)] {
                let decoded = code
                    .decode(&channel(&codeword, flipped, erased, seed + 10), 100)
                    .unwrap();
                assert!(decoded.success, "{flipped} erreurs, {erased} effacements");
                assert_eq!(decoded.bits, codeword);
            }
            // Au-delà de ce que la parité peut combler : un échec annoncé.
            let hopeless = code
                .decode(&channel(&codeword, 0, 5000, seed + 20), 60)
                .unwrap();
            assert!(!hopeless.success);
            assert_eq!(hopeless.iterations, 60);
        }
        assert!(code.decode(&[0.0; 10_000], 10).is_err());
    }

    #[test]
    fn tiny_blocks_decode_too() {
        for (k, m) in [(64, 32), (300, 100), (1000, 250)] {
            let code = LdpcCode::new(k, m, 3);
            let codeword = code.encode(&random_bits(k, 9)).unwrap();
            assert!(code.check(&codeword));
            let decoded = code
                .decode(&channel(&codeword, k / 100, k / 20, 4), 100)
                .unwrap();
            assert!(decoded.success, "{k}, {m}");
            assert_eq!(decoded.bits, codeword);
        }
    }
}
