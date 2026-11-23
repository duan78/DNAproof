//! Contraintes biochimiques portant sur les oligos.
//!
//! Les valeurs par défaut sont celles de l'article DNA Fountain (Erlich et Zielinski, 2017) :
//! contenu en GC de 45 % à 55 % et aucune suite de plus de 3 bases identiques.

use crate::sequence::{gc_content, max_homopolymer};

/// Contraintes appliquées à chaque oligo produit.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DnaConstraints {
    /// Part minimale de G et C.
    pub gc_min: f64,
    /// Part maximale de G et C.
    pub gc_max: f64,
    /// Longueur maximale des suites de bases identiques.
    pub max_homopolymer: usize,
}

/// Bornes de GC par défaut, celles de l'article DNA Fountain (45 % à 55 %).
pub const DEFAULT_GC: (f64, f64) = (0.45, 0.55);

impl Default for DnaConstraints {
    fn default() -> Self {
        Self {
            gc_min: DEFAULT_GC.0,
            gc_max: DEFAULT_GC.1,
            max_homopolymer: 3,
        }
    }
}

/// Erreur pour une borne de GC hors de l'intervalle [0, 1].
fn invalid_bound(name: &str, value: f64) -> crate::DnaError {
    crate::DnaError::InvalidConfig(format!(
        "{name} doit être une proportion entre 0 et 1 (reçu {value})"
    ))
}

impl DnaConstraints {
    /// Contraintes personnalisées.
    pub fn new(gc_min: f64, gc_max: f64, max_homopolymer: usize) -> Self {
        Self {
            gc_min,
            gc_max,
            max_homopolymer,
        }
    }

    /// Vérifie la cohérence des bornes.
    pub fn validate(&self) -> crate::Result<()> {
        let DnaConstraints { gc_min, gc_max, .. } = *self;
        if !(gc_min.is_finite() && (0.0..=1.0).contains(&gc_min)) {
            return Err(invalid_bound("gc_min", gc_min));
        }
        if !(gc_max.is_finite() && (0.0..=1.0).contains(&gc_max)) {
            return Err(invalid_bound("gc_max", gc_max));
        }
        if gc_min >= gc_max {
            return Err(crate::DnaError::InvalidConfig(format!(
                "bornes de GC incohérentes: [{gc_min}, {gc_max}]"
            )));
        }
        if self.max_homopolymer == 0 {
            return Err(crate::DnaError::InvalidConfig(
                "max_homopolymer doit être >= 1".to_string(),
            ));
        }
        Ok(())
    }

    /// Vrai si les bornes de GC sont celles par défaut.
    pub fn gc_is_default(&self) -> bool {
        self.gc_min == DEFAULT_GC.0 && self.gc_max == DEFAULT_GC.1
    }

    /// Vrai si les bornes de GC ne contraignent rien (de 0 à 1).
    pub fn gc_is_free(&self) -> bool {
        self.gc_min == 0.0 && self.gc_max == 1.0
    }

    /// Indique si la séquence satisfait les deux contraintes.
    pub fn satisfied(&self, seq: &[u8]) -> bool {
        if max_homopolymer(seq) > self.max_homopolymer {
            return false;
        }
        let gc = gc_content(seq);
        gc >= self.gc_min && gc <= self.gc_max
    }

    /// Décrit la première contrainte violée, si elle existe.
    pub fn check(&self, seq: &[u8]) -> crate::Result<()> {
        let run = max_homopolymer(seq);
        if run > self.max_homopolymer {
            return Err(crate::DnaError::ConstraintViolation(format!(
                "suite de {} bases identiques (maximum {})",
                run, self.max_homopolymer
            )));
        }
        let gc = gc_content(seq);
        if gc < self.gc_min || gc > self.gc_max {
            return Err(crate::DnaError::ConstraintViolation(format!(
                "contenu en GC de {:.1}% hors de [{:.0}%, {:.0}%]",
                gc * 100.0,
                self.gc_min * 100.0,
                self.gc_max * 100.0
            )));
        }
        Ok(())
    }

    /// Probabilité qu'une séquence aléatoire de `length` nucléotides respecte les contraintes.
    ///
    /// Calculée exactement, par programmation dynamique sur la dernière base, la longueur de la
    /// suite identique en cours et le nombre de G et C. Avec les contraintes de l'article, une
    /// séquence de 150 nt passe 12,5 % du temps.
    pub fn pass_probability(&self, length: usize) -> f64 {
        if length == 0 {
            return 1.0;
        }
        let max_run = self.max_homopolymer.min(length);
        let runs = max_run + 1;
        let stride_run = length + 1;
        let stride_base = runs * stride_run;
        let is_gc = |base: usize| usize::from((1..=2).contains(&base));

        // current[base][run][gc] : nombre de séquences de la longueur courante.
        let mut current = vec![0.0f64; 4 * stride_base];
        for base in 0..4 {
            current[base * stride_base + stride_run + is_gc(base)] = 1.0;
        }
        for _ in 1..length {
            let mut next = vec![0.0f64; 4 * stride_base];
            for base in 0..4 {
                for run in 1..=max_run {
                    for gc in 0..length {
                        let count = current[base * stride_base + run * stride_run + gc];
                        if count == 0.0 {
                            continue;
                        }
                        for next_base in 0..4 {
                            let next_run = if next_base == base { run + 1 } else { 1 };
                            if next_run > max_run {
                                continue;
                            }
                            let index = next_base * stride_base
                                + next_run * stride_run
                                + gc
                                + is_gc(next_base);
                            next[index] += count;
                        }
                    }
                }
            }
            current = next;
        }

        let total: f64 = (0..4)
            .flat_map(|base| (1..=max_run).map(move |run| (base, run)))
            .map(|(base, run)| {
                (0..=length)
                    .filter(|&gc| {
                        let ratio = gc as f64 / length as f64;
                        ratio >= self.gc_min && ratio <= self.gc_max
                    })
                    .map(|gc| current[base * stride_base + run * stride_run + gc])
                    .sum::<f64>()
            })
            .sum();
        total / 4f64.powi(length as i32)
    }

    /// Nombre de candidats à essayer pour en trouver un conforme, ou `None` si l'espace de
    /// recherche ne suffit pas.
    ///
    /// `space` est le nombre de candidats distincts disponibles (graines, nonces). La limite
    /// donnée laisse moins d'une chance sur 10^17 de n'en trouver aucun ; elle vaut pour une
    /// longueur d'oligo `length` donnée.
    pub fn search_limit(&self, length: usize, space: u64) -> Option<u64> {
        let p = self.pass_probability(length);
        if p <= 0.0 || p.is_nan() {
            return None;
        }
        let needed = (40.0 / p).ceil();
        if needed.is_finite() && needed <= space as f64 {
            Some(needed.max(1.0) as u64)
        } else {
            None
        }
    }

    /// Probabilité qu'une séquence aléatoire de `length` nucléotides respecte la seule contrainte
    /// de GC (sans limite sur `length` : au-delà de mille nucléotides, le calcul déborde).
    ///
    /// Sert quand la contrainte sur les suites est tenue par construction, comme par les
    /// trimères du schéma grass.
    pub fn gc_pass_probability(&self, length: usize) -> f64 {
        if length == 0 {
            return 1.0;
        }
        let low = (self.gc_min * length as f64).ceil() as usize;
        let high = (self.gc_max * length as f64).floor() as usize;
        // Poids binomiaux, déjà ramenés à 2^-length : poids(g) = C(length, g) / 2^length.
        let mut weight = 0.5f64.powi(length as i32);
        let mut total = 0.0;
        for gc in 0..=length {
            if gc > 0 {
                weight *= (length - gc + 1) as f64 / gc as f64;
            }
            if (low..=high).contains(&gc) {
                total += weight;
            }
        }
        total
    }

    /// Probabilité qu'aucun de `space` candidats indépendants ne respecte les contraintes.
    pub fn all_fail_probability(&self, length: usize, space: u64) -> f64 {
        (1.0 - self.pass_probability(length)).powf(space as f64)
    }

    /// Message d'erreur pour des contraintes qu'un oligo de `length` nucléotides ne peut satisfaire.
    pub fn too_strict_message(&self, length: usize) -> String {
        let p = self.pass_probability(length);
        let one_in = if p > 0.0 {
            format!("une sur {}", (1.0 / p).round())
        } else {
            "aucune".to_string()
        };
        format!(
            "contraintes trop strictes pour un oligo de {length} nt : {one_in} séquence \
             aléatoire les respecte, et l'espace de recherche ne suffit pas"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constraints_match_paper() {
        let c = DnaConstraints::default();
        assert!((c.gc_min - 0.45).abs() < 1e-12);
        assert!((c.gc_max - 0.55).abs() < 1e-12);
        assert_eq!(c.max_homopolymer, 3);
        assert!(c.gc_is_default() && !c.gc_is_free());
        let free = DnaConstraints::new(0.0, 1.0, 3);
        assert!(free.gc_is_free() && !free.gc_is_default());
        assert!(!DnaConstraints::new(0.40, 0.60, 3).gc_is_default());
    }

    #[test]
    fn pass_probability_matches_the_paper() {
        // Erlich et Zielinski : « only 12.5% of the oligos pass these conditions » pour
        // 150 nt, GC de 45 % à 55 %, suites d'au plus 3 bases.
        let p = DnaConstraints::default().pass_probability(150);
        // Le calcul exact donne 13,3 % : l'ordre de grandeur de l'article, confirmé par tirage.
        assert!((p - 0.125).abs() < 0.01, "{p}");
        // Plus long, plus dur : moins de suites longues admises.
        let long = DnaConstraints::default().pass_probability(152);
        assert!(long < p);
    }

    #[test]
    fn pass_probability_agrees_with_sampling() {
        let constraints = DnaConstraints::new(0.40, 0.60, 3);
        let expected = constraints.pass_probability(40);
        let mut rng = crate::codec::rng::SeededRng::new(5, 6);
        let trials = 200_000u32;
        let passed = (0..trials)
            .filter(|_| {
                let mut raw = [0u8; 40];
                rng.fill_bytes(&mut raw);
                let sequence: Vec<u8> = raw
                    .iter()
                    .map(|&byte| crate::sequence::BASES[byte as usize & 3])
                    .collect();
                constraints.satisfied(&sequence)
            })
            .count();
        let observed = passed as f64 / trials as f64;
        // Écart-type attendu : environ 0,001 ; on accepte six fois cette marge.
        assert!(
            (observed - expected).abs() < 0.006,
            "{observed} vs {expected}"
        );
        assert!((DnaConstraints::default().pass_probability(0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn search_limit_scales_with_strictness_and_refuses_the_impossible() {
        let defaults = DnaConstraints::default();
        // Une limite de quelques centaines d'essais pour une goutte sur huit.
        let limit = defaults.search_limit(152, 1 << 31).unwrap();
        assert!((250..=400).contains(&limit), "{limit}");
        // Suites d'au plus deux bases et GC de 49 % à 51 % : une séquence sur 22 000. Hors de
        // portée de 65 536 candidats, à la portée de 24 bits.
        let narrow = DnaConstraints::new(0.49, 0.51, 2);
        assert!(narrow.search_limit(160, 1 << 16).is_none());
        assert!(narrow.search_limit(160, 1 << 24).is_some());
        // GC d'au moins 99 % : quasiment impossible, quel que soit l'espace.
        let absurd = DnaConstraints::new(0.99, 1.0, 3);
        assert!(absurd.search_limit(152, u64::MAX).is_none());
        assert!(absurd.too_strict_message(152).contains("152 nt"));
        // Aucune contrainte sur les suites : limite minimale.
        let loose = DnaConstraints::new(0.0, 1.0, 1000);
        // La somme flottante vaut 1 à un chiffre près : 40 essais, ou 41 par excès.
        assert!(matches!(loose.search_limit(152, 1 << 16), Some(40..=41)));
        // Avec 47 candidats seulement (les nonces du schéma grass), le risque par brin dépend de
        // la probabilité de passage : une séquence aléatoire de 117 nt en a une sur cinq, une
        // suite où seule la contrainte de GC compte, deux sur trois.
        let iid = defaults.pass_probability(117);
        assert!((iid - 0.2).abs() < 0.03, "{iid}");
        assert!((defaults.all_fail_probability(117, 47) - (1.0 - iid).powi(47)).abs() < 1e-9);
        let gc_only = defaults.gc_pass_probability(117);
        // S'accorde avec le modèle complet quand les suites ne sont pas limitées.
        let free = DnaConstraints::new(0.45, 0.55, 1000);
        assert!(
            (free.pass_probability(117) - gc_only).abs() < 1e-9,
            "{gc_only}"
        );
        assert!((0.70..0.76).contains(&gc_only), "{gc_only}");
        // Avec 47 candidats, le risque par brin devient infime.
        assert!((1.0 - gc_only).powi(47) < 1e-16);
        assert_eq!(defaults.gc_pass_probability(0), 1.0);
        assert!((free.pass_probability(60) - free.gc_pass_probability(60)).abs() < 1e-9);
        assert!((DnaConstraints::new(0.0, 1.0, 3).gc_pass_probability(300) - 1.0).abs() < 1e-9);
        assert_eq!(defaults.all_fail_probability(152, 0), 1.0);
    }

    #[test]
    fn satisfied_and_check_agree() {
        let c = DnaConstraints::default();
        assert!(c.satisfied(b"ACGTTGCAACGTTCGA"));
        assert!(c.check(b"ACGTTGCAACGTTCGA").is_ok());
        assert!(!c.satisfied(b"AAAA"));
        assert!(c.check(b"AAAA").is_err());
        assert!(!c.satisfied(b"GCGCGCGC"));
        assert!(c.check(b"GCGCGCGC").is_err());
    }

    #[test]
    fn validate_rejects_inconsistent_bounds() {
        assert!(DnaConstraints::new(0.6, 0.4, 3).validate().is_err());
        assert!(DnaConstraints::new(0.4, 0.6, 0).validate().is_err());
        assert!(DnaConstraints::new(0.4, 0.6, 3).validate().is_ok());
    }
}
