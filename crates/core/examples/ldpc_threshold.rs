//! Seuils de décodage du code LDPC, sur un canal binaire symétrique et sur un canal à
//! effacements.
//!
//! ```sh
//! cargo run --release -p adn-core --example ldpc_threshold -- [bits_d'information] [bits_de_parité] [tirages] [poids_des_colonnes]
//! ```
//!
//! Sans arguments : 81 920 bits d'information, 20 480 de parité (rendement 0,8), dix tirages par
//! point, quatre contrôles par bit d'information. Les chiffres de `docs/PERFORMANCE.md` viennent
//! d'ici.

use adn_core::codec::rng::SeededRng;
use adn_core::ldpc::LdpcCode;
use std::time::Instant;

fn random_bits(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SeededRng::new(seed, 0x41);
    (0..len).map(|_| rng.below(2) as u8).collect()
}

/// `trials` tirages sur un même point du canal : des erreurs (`erased` à faux) ou des effacements.
fn run(code: &LdpcCode, trials: u64, percent: f64, erased: bool) -> (usize, f64) {
    let k = code.k();
    let mut rng = SeededRng::new(if erased { 78 } else { 77 }, 0x44);
    let (mut successes, mut iterations) = (0, 0);
    for seed in 0..trials {
        let codeword = code.encode(&random_bits(k, seed)).expect("bonne longueur");
        let llrs: Vec<f32> = codeword
            .iter()
            .map(|&bit| match (erased, rng.next_f64() < percent / 100.0) {
                (true, true) => 0.0,
                (_, false) => 4.0 - 8.0 * bit as f32,
                (_, true) => 8.0 * bit as f32 - 4.0,
            })
            .collect();
        let decoded = code.decode(&llrs, 100).expect("bonne longueur");
        successes += usize::from(decoded.success);
        iterations += decoded.iterations;
    }
    (successes, iterations as f64 / trials as f64)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |position: usize, default: usize| -> usize {
        args.get(position)
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    let (k, m, weight) = (number(0, 81_920), number(1, 20_480), number(3, 4));
    let trials = number(2, 10) as u64;
    let started = Instant::now();
    let code = LdpcCode::with_column_weight(k, m, weight, 5);
    println!(
        "{} bits d'information, {} de parité (rendement {:.3}), {} contrôles par bit ; construction {} ms",
        k,
        m,
        k as f64 / code.codeword_len() as f64,
        weight,
        started.elapsed().as_millis()
    );
    for percent in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0] {
        let (successes, iterations) = run(&code, trials, percent, false);
        println!(
            "erreurs {percent:.1} % : {successes}/{trials} restitués, {iterations:.1} itérations en moyenne"
        );
    }
    for percent in [16.0, 20.0, 25.0, 30.0, 35.0, 40.0] {
        let (successes, iterations) = run(&code, trials, percent, true);
        println!(
            "effacements {percent:.1} % : {successes}/{trials} restitués, {iterations:.1} itérations en moyenne"
        );
    }
    let codeword = code.encode(&random_bits(k, 1)).expect("bonne longueur");
    let clean: Vec<f32> = codeword.iter().map(|&bit| 4.0 - 8.0 * bit as f32).collect();
    let started = Instant::now();
    let runs = 20;
    for _ in 0..runs {
        std::hint::black_box(code.decode(&clean, 100).expect("bonne longueur"));
    }
    println!(
        "{:.2} ms par décodage d'un mot de code intact",
        started.elapsed().as_secs_f64() * 1000.0 / runs as f64
    );
}
