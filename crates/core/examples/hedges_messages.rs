//! Le code à hachis pris isolément : ce que la recherche rend de lectures abîmées.
//!
//! ```sh
//! cargo run --release -p adn-core --example hedges_messages
//! ```
//!
//! Pour chaque rendement, à son taux nominal d'erreurs puis au-delà, dix mille brins sont abîmés
//! — autant de substitutions, d'insertions et de suppressions — et relus par la recherche, sans
//! vote ni code externe. Une lecture « fausse » est résolue, mais avec un autre message que
//! celui du brin : c'est ce que le vote et le code externe doivent arrêter. Une lecture « à
//! budget » épuise les 65 536 hypothèses : elle coûte ce que coûte une lecture étrangère. Les
//! chiffres de `docs/PERFORMANCE.md` viennent d'ici.

use adn_core::codec::rng::SeededRng;
use adn_core::hedges::{decode, encode, Layout, Outcome, Rate, Search};
use std::time::Instant;

const BASES: [u8; 4] = *b"ACGT";
const READS: u32 = 10_000;

/// (rendement, taux de chaque sorte d'erreur par base) : le taux nominal de chaque rendement,
/// puis ce qu'il traverse encore.
const POINTS: [(Rate, f64); 10] = [
    (Rate::THREE_QUARTERS, 0.0033),
    (Rate::THREE_QUARTERS, 0.005),
    (Rate::THREE_QUARTERS, 0.01),
    (Rate::HALF, 0.01),
    (Rate::HALF, 0.02),
    (Rate::THIRD, 0.02),
    (Rate::THIRD, 0.033),
    (Rate::QUARTER, 0.033),
    (Rate::QUARTER, 0.04),
    (Rate::QUARTER, 0.05),
];

/// Une lecture abîmée des trois façons : chaque base est remplacée, ou précède une base insérée,
/// ou disparaît, avec le taux `each`.
fn damage(dna: &[u8], each: f64, rng: &mut SeededRng) -> Vec<u8> {
    let mut read = Vec::with_capacity(dna.len() + 8);
    for &base in dna {
        let roll = rng.next_f64();
        if roll < each {
            let rank = BASES
                .iter()
                .position(|&candidate| candidate == base)
                .unwrap_or(0);
            read.push(BASES[(rank + 1 + rng.below(3) as usize) % 4]);
        } else if roll < 2.0 * each {
            read.push(BASES[rng.below(4) as usize]);
            read.push(base);
        } else if roll < 3.0 * each {
            // La base disparaît.
        } else {
            read.push(base);
        }
    }
    read
}

fn main() {
    println!(
        "{:>9}  {:>7}  {:>7}  {:>7}  {:>6}  {:>8}  {:>7}  {:>8}  {:>7}",
        "rendement",
        "erreurs",
        "exactes",
        "fausses",
        "perdues",
        "a_budget",
        "rivales",
        "refusees",
        "duree"
    );
    for (rate, each) in POINTS {
        let layout = Layout::strand(rate);
        let search = Search::default();
        let mut rng = SeededRng::new(7, 0x1234);
        let (mut exact, mut wrong, mut lost) = (0, 0, 0);
        let (mut exhausted, mut rival, mut invalid) = (0, 0, 0);
        let started = Instant::now();
        for _ in 0..READS {
            let mut free = vec![0u8; layout.free_bytes()];
            rng.fill_bytes(&mut free);
            let dna = encode(&layout, &free);
            let decoded = decode(&layout, &damage(&dna, each, &mut rng), search);
            match decoded.outcome {
                Outcome::Solved if decoded.message.as_deref() == Some(free.as_slice()) => {
                    exact += 1
                }
                Outcome::Solved => wrong += 1,
                Outcome::Lost => lost += 1,
                Outcome::Exhausted => exhausted += 1,
                Outcome::Rival => rival += 1,
                Outcome::Invalid => invalid += 1,
            }
        }
        println!(
            "{:>9}  {:>6.1} %  {:>7}  {:>7}  {:>6}  {:>8}  {:>7}  {:>8}  {:>6.1} s",
            rate.name(),
            300.0 * each,
            exact,
            wrong,
            lost,
            exhausted,
            rival,
            invalid,
            started.elapsed().as_secs_f64()
        );
    }
}
