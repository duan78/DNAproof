//! Distribution soliton robuste (Luby, 2002).
//!
//! C'est la distribution du degré des gouttes d'un code LT. Elle reprend la définition mise en
//! œuvre dans le logiciel de référence de DNA Fountain :
//!
//! - `S = c · ln(K/δ) · √K` : nombre espéré de gouttes de degré 1 ;
//! - pivot `⌊K/S⌋` ;
//! - `ρ(1) = 1/K`, `ρ(d) = 1/(d·(d−1))` ;
//! - `τ(d) = (S/K)/d` pour `d < pivot`, `τ(pivot) = (S/K)·ln(S/δ)`, nul ensuite ;
//! - `μ(d) = (ρ(d) + τ(d)) / Z` où `Z` normalise la somme à 1.
//!
//! Avec les paramètres de l'article (`c = 0,025`, `δ = 0,001`), le décodage d'un fichier à
//! 67 088 blocs a réussi avec 7 % de gouttes en trop.

use crate::codec::rng::SeededRng;
use crate::error::{DnaError, Result};

#[derive(Clone, Debug)]
pub struct RobustSoliton {
    /// Fonction de répartition : `cdf[d - 1]` vaut la probabilité que le degré soit inférieur ou
    /// égal à `d`.
    cdf: Vec<f64>,
}

impl RobustSoliton {
    /// Construit la distribution pour `k` blocs.
    pub fn new(k: u32, c: f64, delta: f64) -> Result<Self> {
        if k == 0 {
            return Err(DnaError::InvalidConfig(
                "le nombre de blocs doit être au moins 1".to_string(),
            ));
        }
        if !(c.is_finite() && c > 0.0) {
            return Err(DnaError::InvalidConfig(format!(
                "le paramètre soliton c doit être un nombre fini et positif (reçu {c})"
            )));
        }
        if !(delta.is_finite() && delta > 0.0 && delta < 1.0) {
            return Err(DnaError::InvalidConfig(format!(
                "le paramètre soliton δ doit être un nombre compris strictement entre 0 et 1 (reçu {delta})"
            )));
        }
        let kf = f64::from(k);
        let s = c * (kf / delta).ln() * kf.sqrt();
        // Un logarithme négatif ou nul donnerait des poids négatifs : configuration absurde.
        let log_s = (s / delta).ln();
        if !log_s.is_finite() || log_s <= 0.0 {
            return Err(DnaError::InvalidConfig(format!(
                "paramètres soliton incohérents pour {k} blocs: c = {c}, δ = {delta}"
            )));
        }
        // Pour très peu de blocs, S peut dépasser K : le pivot reste alors dans [1, K].
        let pivot = (kf / s).floor().clamp(1.0, kf) as usize;

        let mut cdf = Vec::with_capacity(k as usize);
        let mut cumulative = 0.0;
        for d in 1..=k as usize {
            let df = d as f64;
            let rho = if d == 1 {
                1.0 / kf
            } else {
                1.0 / (df * (df - 1.0))
            };
            let tau = if d < pivot {
                s / kf / df
            } else if d == pivot {
                s / kf * log_s
            } else {
                0.0
            };
            cumulative += rho + tau;
            cdf.push(cumulative);
        }
        for value in &mut cdf {
            *value /= cumulative;
        }
        if let Some(last) = cdf.last_mut() {
            *last = 1.0;
        }
        Ok(Self { cdf })
    }

    /// Nombre de blocs de la distribution.
    pub fn k(&self) -> u32 {
        self.cdf.len() as u32
    }

    /// Degré le plus probable.
    pub fn mode(&self) -> usize {
        let mut best = 1;
        let mut best_mass = 0.0;
        let mut previous = 0.0;
        for (d, &value) in self.cdf.iter().enumerate() {
            if value - previous > best_mass {
                best_mass = value - previous;
                best = d + 1;
            }
            previous = value;
        }
        best
    }

    /// Degré moyen.
    pub fn mean(&self) -> f64 {
        let mut previous = 0.0;
        let mut mean = 0.0;
        for (d, &value) in self.cdf.iter().enumerate() {
            mean += (d + 1) as f64 * (value - previous);
            previous = value;
        }
        mean
    }

    /// Probabilité qu'une goutte soit de degré 1.
    pub fn probability_degree_one(&self) -> f64 {
        self.cdf[0]
    }

    /// Tire un degré.
    pub fn sample(&self, rng: &mut SeededRng) -> u32 {
        let u = rng.next_f64();
        // Premier indice dont la valeur cumulée dépasse u.
        let index = self.cdf.partition_point(|&value| value <= u);
        (index as u32 + 1).min(self.k())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> (u32, f64, f64) {
        (67_088, 0.025, 0.001)
    }

    #[test]
    fn paper_parameters_give_small_mean_degree() {
        let (k, c, delta) = params();
        let distribution = RobustSoliton::new(k, c, delta).unwrap();
        // S = c·ln(K/δ)·√K ≈ 116,7 gouttes de degré 1 attendues parmi K, à la normalisation
        // près (facteur 1,02).
        let s = c * (f64::from(k) / delta).ln() * f64::from(k).sqrt();
        let expected = s / f64::from(k);
        let observed = distribution.probability_degree_one();
        assert!(
            (observed - expected).abs() < 0.05 * expected,
            "{observed} vs {expected}"
        );
        // Valeurs mesurées : à changer seulement si la définition change, ce qui casserait
        // la lecture des archives existantes.
        assert_eq!(distribution.mode(), 2);
        assert!(
            (distribution.mean() - 23.57).abs() < 0.01,
            "{}",
            distribution.mean()
        );
    }

    #[test]
    fn mean_degree_grows_logarithmically() {
        // Mille fois plus de blocs : moins de deux fois plus de degré moyen (15,3 puis 28,2).
        let small = RobustSoliton::new(1_000, 0.025, 0.001).unwrap().mean();
        let large = RobustSoliton::new(1_000_000, 0.025, 0.001).unwrap().mean();
        assert!(large / small < 2.0 && large > small, "{small} -> {large}");
        let tiny = RobustSoliton::new(10, 0.025, 0.001).unwrap().mean();
        assert!(tiny < small);
    }

    #[test]
    fn sampling_stays_in_range_and_reproduces_distribution() {
        let (k, c, delta) = params();
        let distribution = RobustSoliton::new(k, c, delta).unwrap();
        let mut rng = SeededRng::new(1, 1);
        let mut degree_one = 0u64;
        let trials = 200_000u64;
        let mut max = 0;
        for _ in 0..trials {
            let d = distribution.sample(&mut rng);
            assert!((1..=k).contains(&d));
            max = max.max(d);
            if d == 1 {
                degree_one += 1;
            }
        }
        let observed = degree_one as f64 / trials as f64;
        let expected = distribution.probability_degree_one();
        // Écart-type attendu : 9e-5 ; on accepte quatre fois cette marge.
        assert!(
            (observed - expected).abs() < 0.25 * expected,
            "{observed} vs {expected}"
        );
        assert!(
            max > 100,
            "les gouttes de fort degré doivent exister: {max}"
        );
    }

    #[test]
    fn works_for_tiny_and_round_k() {
        for k in [1u32, 2, 3, 4, 10, 100, 255, 65_536] {
            let distribution = RobustSoliton::new(k, 0.025, 0.001).unwrap();
            assert_eq!(distribution.k(), k);
            assert!((distribution.cdf.last().unwrap() - 1.0).abs() < 1e-12);
            let mut rng = SeededRng::new(3, 1);
            for _ in 0..1_000 {
                assert!((1..=k).contains(&distribution.sample(&mut rng)));
            }
        }
    }

    #[test]
    fn rejects_absurd_parameters() {
        assert!(RobustSoliton::new(0, 0.025, 0.001).is_err());
        assert!(RobustSoliton::new(100, 0.0, 0.001).is_err());
        assert!(RobustSoliton::new(100, 0.025, 1.0).is_err());
        assert!(RobustSoliton::new(100, f64::NAN, 0.001).is_err());
        assert!(RobustSoliton::new(100, 0.025, f64::INFINITY).is_err());
    }
}
