//! Code convolutif binaire, longueur de contrainte 7, rendement ½, polynômes (171, 133) en octal.
//!
//! C'est le code convolutif le plus répandu, celui des normes de communication spatiale. Un bit
//! d'entrée donne deux bits de code ; l'encodeur garde les six bits précédents en mémoire, et le
//! décodeur exploite ce qui sépare deux mots de code : dix bits au moins (distance libre).
//!
//! Le message est terminé par six zéros, qui ramènent l'encodeur à l'état nul : le décodeur sait
//! où finir.
//!
//! - [`encode`] : les bits de code du message.
//! - [`viterbi`] : le chemin le plus probable du treillis, pour des entrées souples (rapports de
//!   vraisemblance) et une décision dure.
//! - [`bcjr`] : le rapport de vraisemblance a posteriori de chaque bit d'information, sur le même
//!   treillis (algorithme log-MAP), dont le signe est la décision au sens du maximum a
//!   posteriori.
//!
//! Les rapports de vraisemblance valent `ln P(bit = 0) / P(bit = 1)` : positif pour un bit à zéro.

use std::sync::OnceLock;

/// Longueur de contrainte, en bits : le bit présent et les six gardés en mémoire.
pub const CONSTRAINT: usize = 7;
/// Polynômes générateurs, en octal.
pub const GENERATORS: [u64; 2] = [0o171, 0o133];
/// Bits de queue ajoutés au message.
pub const TAIL: usize = CONSTRAINT - 1;
/// Nombre d'états du treillis.
pub const STATES: usize = 1 << TAIL;
/// Distance libre du code : poids du plus léger mot de code non nul.
pub const FREE_DISTANCE: usize = 10;

/// État suivant et bits de code de chaque transition, indexées par `état × 2 + bit d'entrée`.
/// Le registre compte sept bits : l'entrée en poids fort, puis les six précédentes de la plus
/// récente à la plus ancienne. Un un isolé en entrée fait donc lire chaque polynôme de gauche à
/// droite.
static TABLES: ([u8; 2 * STATES], [u8; 2 * STATES]) = {
    const fn parity(mut value: u64) -> u64 {
        value ^= value >> 32;
        value ^= value >> 16;
        value ^= value >> 8;
        value ^= value >> 4;
        value ^= value >> 2;
        value ^= value >> 1;
        value & 1
    }
    let mut next = [0u8; 2 * STATES];
    let mut output = [0u8; 2 * STATES];
    let mut edge = 0;
    while edge < 2 * STATES {
        let state = (edge >> 1) as u64;
        let input = (edge & 1) as u64;
        let register = (input << TAIL) | state;
        next[edge] = ((state >> 1) | (input << (TAIL - 1))) as u8;
        let mut generator = 0;
        while generator < 2 {
            output[edge] |= (parity(register & GENERATORS[generator]) << generator) as u8;
            generator += 1;
        }
        edge += 1;
    }
    (next, output)
};

/// Nombre de bits de code d'un message de `bits` bits d'information.
pub const fn coded_len(bits: usize) -> usize {
    2 * (bits + TAIL)
}

/// Bits de code du message, terminé par la queue. Chaque bit vaut 0 ou 1 : le bit de poids
/// faible du premier polynôme, puis celui du second.
pub fn encode(bits: &[u8]) -> Vec<u8> {
    let mut state = 0usize;
    let mut coded = Vec::with_capacity(coded_len(bits.len()));
    for &bit in bits.iter().chain(std::iter::repeat_n(&0, TAIL)) {
        let edge = (state << 1) | usize::from(bit & 1);
        coded.push(TABLES.1[edge] & 1);
        coded.push(TABLES.1[edge] >> 1);
        state = usize::from(TABLES.0[edge]);
    }
    coded
}

/// Coût d'une transition pour un couple de rapports de vraisemblance : chaque bit de code à un
/// coûte le rapport de vraisemblance de sa position, un bit à zéro ne coûte rien. C'est le
/// négatif du log de vraisemblance, à une constante près par position.
#[inline]
fn cost(edge: usize, first: f32, second: f32) -> f32 {
    let output = TABLES.1[edge];
    (if output & 1 == 1 { first } else { 0.0 }) + (if output >> 1 == 1 { second } else { 0.0 })
}

/// Nombre de bits d'information que portent `llrs`, rapportés deux par deux ; `None` si la
/// longueur ne s'y prête pas.
fn message_len(llrs: &[f32]) -> Option<usize> {
    let coded = llrs.len();
    (coded.is_multiple_of(2) && coded >= 2 * TAIL).then(|| coded / 2 - TAIL)
}

/// Le chemin le plus probable du treillis : les bits d'information décidés.
///
/// Chaque position de code apporte son rapport de vraisemblance ; un bit à un coûte le rapport
/// de sa position, et le chemin de moindre coût l'emporte. `None` si `llrs` ne porte pas un
/// nombre entier de couples, ou moins que la queue.
pub fn viterbi(llrs: &[f32]) -> Option<Vec<u8>> {
    let bits = message_len(llrs)?;
    let steps = bits + TAIL;
    // Coût du meilleur chemin menant à chaque état ; infini pour un état inatteignable.
    let mut previous = vec![f32::INFINITY; STATES];
    previous[0] = 0.0;
    let mut current = vec![0f32; STATES];
    let mut decisions = vec![0u64; steps];
    for (step, pair) in llrs.as_chunks::<2>().0.iter().enumerate() {
        let (first, second) = (pair[0], pair[1]);
        let mut chosen = 0u64;
        for (state, metric) in current.iter_mut().enumerate() {
            // Le bit d'entrée est le poids fort de l'état atteint. Les deux prédécesseurs ont
            // les cinq entrées suivantes en commun ; ils diffèrent par la plus ancienne.
            let input = state >> (TAIL - 1);
            let older = (state << 1) % STATES;
            let from_zero = previous[older] + cost((older << 1) | input, first, second);
            let from_one = previous[older | 1] + cost(((older | 1) << 1) | input, first, second);
            *metric = if from_one < from_zero {
                chosen |= 1 << state;
                from_one
            } else {
                from_zero
            };
        }
        decisions[step] = chosen;
        std::mem::swap(&mut previous, &mut current);
    }
    // Retour sur le chemin, depuis l'état nul final.
    let mut state = 0usize;
    let mut decoded = vec![0u8; bits];
    for step in (0..steps).rev() {
        let oldest = ((decisions[step] >> state) & 1) as usize;
        if step < bits {
            decoded[step] = (state >> (TAIL - 1)) as u8;
        }
        state = ((state << 1) % STATES) | oldest;
    }
    Some(decoded)
}

/// Pas de la table de correction de `max_star`.
const CORRECTION_STEP: f32 = 1.0 / 256.0;
/// Au-delà, la correction est nulle : `ln(1 + e^-20)` vaut deux milliardièmes.
const CORRECTION_MAX: f32 = 20.0;

/// `ln(1 + e^-d)` pour `d` de 0 à [`CORRECTION_MAX`], par pas de [`CORRECTION_STEP`].
fn corrections() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..(CORRECTION_MAX / CORRECTION_STEP) as usize)
            .map(|index| {
                let distance = index as f32 * CORRECTION_STEP;
                (-distance).exp().ln_1p()
            })
            .collect()
    })
}

/// `ln(e^a + e^b)`, par table : le plus grand des deux, corrigé d'un terme qui ne dépend que de
/// leur écart.
#[inline]
fn max_star(a: f32, b: f32) -> f32 {
    if a < b {
        max_star(b, a)
    } else {
        let gap = a - b;
        if gap >= CORRECTION_MAX {
            a
        } else {
            a + corrections()[(gap / CORRECTION_STEP) as usize]
        }
    }
}

/// Décisions et sorties souples d'un décodage sur le treillis.
#[derive(Clone, Debug, PartialEq)]
pub struct Soft {
    /// Bits décidés : le signe du rapport a posteriori, bit à bit.
    pub bits: Vec<u8>,
    /// Rapport de vraisemblance a posteriori de chaque bit d'information.
    pub llrs: Vec<f32>,
}

/// Rapports de vraisemblance a posteriori des bits d'information (algorithme log-MAP, dit BCJR),
/// et leurs décisions.
///
/// Chaque position de code apporte son rapport de vraisemblance, chaque bit d'information un
/// *a priori* (`prior`, nul pour un bit sans préférence). `None` si les longueurs ne se
/// correspondent pas.
pub fn bcjr(llrs: &[f32], prior: &[f32]) -> Option<Soft> {
    let bits = message_len(llrs)?;
    if prior.len() != bits {
        return None;
    }
    let steps = bits + TAIL;
    // Poids des transitions, par pas : la log-vraisemblance de leurs bits de code et de leur bit
    // d'entrée, à une constante près par pas (elle disparaît des différences). Rapports et a
    // priori sont pris par moitié, pour que les sorties soient des rapports de vraisemblance.
    let mut weights = vec![[0f32; 2 * STATES]; steps];
    for (step, weights) in weights.iter_mut().enumerate() {
        let (first, second) = (llrs[2 * step] / 2.0, llrs[2 * step + 1] / 2.0);
        let apriori = (if step < bits { prior[step] } else { 0.0 }) / 2.0;
        for (edge, weight) in weights.iter_mut().enumerate() {
            *weight = apriori * (1.0 - 2.0 * (edge & 1) as f32) - cost(edge, first, second);
        }
    }

    // En avant : alpha[t][état] pondère les chemins qui mènent à chaque état en t pas.
    let mut alpha = vec![vec![f32::NEG_INFINITY; STATES]; steps + 1];
    alpha[0][0] = 0.0;
    for step in 0..steps {
        let (before, after) = alpha.split_at_mut(step + 1);
        let from = &before[step];
        let to = &mut after[0];
        for (edge, &weight) in weights[step].iter().enumerate() {
            let target = usize::from(TABLES.0[edge]);
            to[target] = max_star(to[target], from[edge >> 1] + weight);
        }
    }
    // En arrière : beta[t][état] pondère la suite, de l'état en t pas jusqu'à l'état nul final.
    let mut beta = vec![vec![f32::NEG_INFINITY; STATES]; steps + 1];
    beta[steps][0] = 0.0;
    for step in (0..steps).rev() {
        let (before, after) = beta.split_at_mut(step + 1);
        let from = &mut before[step];
        let to = &after[0];
        for (edge, &weight) in weights[step].iter().enumerate() {
            from[edge >> 1] = max_star(from[edge >> 1], weight + to[usize::from(TABLES.0[edge])]);
        }
    }
    // Posteriori : le meilleur chemin qui passe par un bit à zéro, face à celui qui passe par ce
    // bit à un.
    let mut soft = Soft {
        bits: vec![0; bits],
        llrs: vec![0f32; bits],
    };
    for step in 0..bits {
        let mut best = [f32::NEG_INFINITY; 2];
        for (edge, &weight) in weights[step].iter().enumerate() {
            let through =
                alpha[step][edge >> 1] + weight + beta[step + 1][usize::from(TABLES.0[edge])];
            best[edge & 1] = max_star(best[edge & 1], through);
        }
        soft.llrs[step] = best[0] - best[1];
        soft.bits[step] = usize::from(best[1] > best[0]) as u8;
    }
    Some(soft)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::rng::SeededRng;

    fn random_bits(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = SeededRng::new(seed, 0x40);
        (0..len).map(|_| rng.below(2) as u8).collect()
    }

    /// Rapports de vraisemblance d'un mot de code lu avec une confiance `confidence`, certaines
    /// positions étant retournées.
    fn channel(coded: &[u8], flipped: &[usize], confidence: f32) -> Vec<f32> {
        coded
            .iter()
            .enumerate()
            .map(|(position, &bit)| {
                let bit = bit ^ flipped.contains(&position) as u8;
                (1.0 - 2.0 * bit as f32) * confidence
            })
            .collect()
    }

    #[test]
    fn the_code_is_the_standard_one() {
        assert_eq!(STATES, 64);
        assert_eq!(GENERATORS, [121, 91]);
        assert_eq!(coded_len(0), 12);
        assert_eq!(coded_len(298), 608);
        // Le message nul donne le mot de code nul.
        assert!(encode(&[0; 50]).iter().all(|&bit| bit == 0));
        assert_eq!(encode(&[]), vec![0; 12]);
        // Un un isolé : les deux polynômes, lus de gauche à droite, enlacés.
        assert_eq!(encode(&[1]), vec![1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 0, 1, 1, 1]);
        // La queue ramène à l'état nul : deux messages distincts ont des mots de code distincts.
        let first = encode(&random_bits(60, 1));
        let second = encode(&random_bits(60, 2));
        assert_ne!(first, second);
    }

    #[test]
    fn free_distance_is_ten() {
        // Poids du plus léger mot de code non nul, à retour à l'état nul, sur tout message de
        // seize bits.
        let mut lightest = usize::MAX;
        for value in 1..1u32 << 16 {
            let bits: Vec<u8> = (0..16).map(|shift| ((value >> shift) & 1) as u8).collect();
            let coded = encode(&bits);
            let weight = coded.iter().filter(|&&bit| bit == 1).count();
            lightest = lightest.min(weight);
        }
        assert_eq!(lightest, FREE_DISTANCE);
    }

    #[test]
    fn viterbi_restores_a_clean_message_and_refuses_bad_lengths() {
        let bits = random_bits(200, 3);
        let coded = encode(&bits);
        let llrs: Vec<f32> = coded.iter().map(|&bit| 1.0 - 2.0 * bit as f32).collect();
        assert_eq!(viterbi(&llrs).unwrap(), bits);
        assert_eq!(message_len(&llrs), Some(200));
        assert_eq!(message_len(&llrs[..llrs.len() - 1]), None);
        assert_eq!(message_len(&llrs[..10]), None);
        assert!(viterbi(&llrs[..11]).is_none());
        assert_eq!(viterbi(&[1.0; 12]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn viterbi_resists_bursts_shorter_than_half_the_free_distance() {
        let bits = random_bits(200, 7);
        let coded = encode(&bits);
        let total = coded.len();
        // Quatre bits consécutifs retournés, n'importe où : toujours corrigés. Un chemin
        // concurrent pèse dix bits de plus, dont quatre au plus sont accusés à tort.
        for start in 0..total - 4 {
            let burst = [start, start + 1, start + 2, start + 3];
            assert_eq!(
                viterbi(&channel(&coded, &burst, 4.0)).unwrap(),
                bits,
                "salve en {start}"
            );
        }
        // Huit : il en faut la moitié pour tromper le décodeur ; ça arrive souvent.
        let fooled = (0..total - 8)
            .filter(|&start| {
                let burst: Vec<usize> = (start..start + 8).collect();
                viterbi(&channel(&coded, &burst, 4.0)).unwrap() != bits
            })
            .count();
        assert!(fooled > total / 4, "{fooled}");
    }

    #[test]
    fn viterbi_resists_scattered_errors_up_to_a_point() {
        let bits = random_bits(200, 17);
        let coded = encode(&bits);
        let total = coded.len();
        for (rate, low, high) in [(0.01, 28, 30), (0.10, 0, 8)] {
            let mut rng = SeededRng::new(23, 24);
            let mut restored = 0;
            for _ in 0..30 {
                let flipped: Vec<usize> = (0..total).filter(|_| rng.next_f64() < rate).collect();
                restored += usize::from(viterbi(&channel(&coded, &flipped, 4.0)).unwrap() == bits);
            }
            assert!((low..=high).contains(&restored), "{rate} : {restored}");
        }
    }

    #[test]
    fn soft_inputs_beat_hard_decisions() {
        let bits = random_bits(200, 19);
        let coded = encode(&bits);
        let mut rng = SeededRng::new(5, 18);
        // Bruit gaussien d'écart type 0,63 sur des symboles ±1 : 5,6 % des décisions dures sont
        // fausses.
        let sigma = 0.63;
        let mut hard_restored = 0;
        let mut soft_restored = 0;
        for _ in 0..200 {
            let received: Vec<f32> = coded
                .iter()
                .map(|&bit| {
                    let first = rng.next_f64().max(1e-12) as f32;
                    let second = rng.next_f64() as f32;
                    let noise =
                        (-2.0 * first.ln()).sqrt() * (2.0 * std::f32::consts::PI * second).cos();
                    1.0 - 2.0 * bit as f32 + sigma * noise
                })
                .collect();
            let soft: Vec<f32> = received
                .iter()
                .map(|value| 2.0 * value / (sigma * sigma))
                .collect();
            let hard: Vec<f32> = soft.iter().map(|value| value.signum()).collect();
            hard_restored += usize::from(viterbi(&hard).unwrap() == bits);
            soft_restored += usize::from(viterbi(&soft).unwrap() == bits);
        }
        assert!(
            soft_restored > hard_restored + 10,
            "souple {soft_restored}, dur {hard_restored}"
        );
    }

    #[test]
    fn bcjr_gives_decisions_and_reliabilities() {
        let bits = random_bits(200, 29);
        let coded = encode(&bits);
        // Canal sain : les décisions sont exactes, les rapports fortement en faveur des bits
        // émis.
        let clean = channel(&coded, &[], 3.0);
        let soft = bcjr(&clean, &[0.0; 200]).unwrap();
        assert_eq!(soft.bits, bits);
        for (position, (&bit, &llr)) in bits.iter().zip(&soft.llrs).enumerate() {
            assert_eq!(usize::from(llr < 0.0), usize::from(bit), "bit {position}");
            assert!(llr.abs() > 6.0, "bit {position} : {llr}");
        }
        // Une salve de quatre erreurs : corrigée, comme par Viterbi, et le bit le plus touché
        // garde le rapport le plus faible.
        let burst: Vec<usize> = (150..154).collect();
        let soft = bcjr(&channel(&coded, &burst, 3.0), &[0.0; 200]).unwrap();
        assert_eq!(soft.bits, bits);
        assert_eq!(soft.bits, viterbi(&channel(&coded, &burst, 3.0)).unwrap());
        let weakest = soft
            .llrs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
            .map(|(position, _)| position)
            .unwrap();
        assert!((70..=80).contains(&weakest), "{weakest}");
        // Des positions effacées (rapport nul) : la décision vient du code.
        let mut erased = channel(&coded, &[], 3.0);
        for value in &mut erased[100..116] {
            *value = 0.0;
        }
        assert_eq!(bcjr(&erased, &[0.0; 200]).unwrap().bits, bits);
        // Un a priori fautif et affirmé l'emporte sur le canal, pour un bit isolé.
        let mut prior = vec![0f32; 200];
        prior[100] = if bits[100] == 0 { -40.0 } else { 40.0 };
        let soft = bcjr(&channel(&coded, &[], 3.0), &prior).unwrap();
        assert_ne!(soft.bits[100], bits[100]);
        // Longueurs incohérentes.
        assert!(bcjr(&clean, &[0.0; 199]).is_none());
        assert!(bcjr(&clean[..clean.len() - 1], &[0.0; 200]).is_none());
    }
}
