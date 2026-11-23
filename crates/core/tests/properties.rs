//! Tests de propriétés : allers-retours sur des entrées aléatoires.
//!
//! Chaque cas encode et décode : 256 cas en mode release, 24 en débogage, 16 avec
//! `ADN_FAST_TESTS=1`.

use adn_core::codec::rng::SeededRng;
use adn_core::{decode, encode, DecodeOptions, DnaConstraints, EncodeOptions, Scheme};
use proptest::prelude::*;

fn sample(kind: u8, len: usize, seed: u64) -> Vec<u8> {
    match kind % 4 {
        0 => {
            let mut rng = SeededRng::new(seed, 1);
            let mut out = vec![0u8; len];
            rng.fill_bytes(&mut out);
            out
        }
        1 => {
            // Texte : peu de valeurs différentes.
            let mut rng = SeededRng::new(seed, 2);
            (0..len)
                .map(|_| b"acgt et adn, urgeon. "[rng.below(21) as usize])
                .collect()
        }
        2 => vec![seed as u8; len],
        _ => (0..len)
            .map(|i| (i.wrapping_add(seed as usize) % 251) as u8)
            .collect(),
    }
}

fn fail(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}

fn run(
    scheme_index: u8,
    kind: u8,
    len: usize,
    seed: u64,
    shuffle: bool,
) -> Result<(), TestCaseError> {
    let scheme = Scheme::ALL[usize::from(scheme_index % 2)];
    let data = sample(kind, len, seed);
    let encoded = encode(&data, &EncodeOptions::for_scheme(scheme))
        .map_err(|e| fail(format!("encode: {e}")))?;
    let constraints = DnaConstraints::default();
    for oligo in &encoded.oligos {
        constraints
            .check(oligo)
            .map_err(|e| fail(format!("oligo non conforme: {e}")))?;
    }
    let mut reads = encoded.oligos.clone();
    if shuffle {
        let mut rng = SeededRng::new(seed, 3);
        for i in (1..reads.len()).rev() {
            reads.swap(i, rng.below(i as u64 + 1) as usize);
        }
    }
    let decoded =
        decode(&reads, &DecodeOptions::default()).map_err(|e| fail(format!("decode: {e}")))?;
    if decoded.data == data && decoded.scheme == scheme {
        Ok(())
    } else {
        Err(fail("fichier reconstitué différent".to_string()))
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        if std::env::var_os("ADN_FAST_TESTS").is_some() {
            16
        } else if cfg!(debug_assertions) {
            24
        } else {
            256
        }
    ))]

    #[test]
    fn roundtrip_on_random_inputs(scheme_index in 0u8..2, kind in 0u8..4, len in 0usize..2_000, seed in any::<u64>(), shuffle in any::<bool>()) {
        run(scheme_index, kind, len, seed, shuffle)?;
    }

    #[test]
    fn roundtrip_with_random_constraints(
        kind in any::<u8>(),
        len in 1usize..600,
        seed in any::<u64>(),
        gc_low in 0.40f64..0.50,
        width in 0.05f64..0.15f64,
        max_run in 3usize..=6,
    ) {
        let constraints = DnaConstraints::new(gc_low, (gc_low + width).min(1.0), max_run);
        let data = sample(kind, len, seed);
        for scheme in Scheme::ALL {
            let options = EncodeOptions {
                constraints,
                ..EncodeOptions::for_scheme(scheme)
            };
            let encoded = encode(&data, &options).map_err(|e| fail(format!("encode: {e}")))?;
            for oligo in &encoded.oligos {
                constraints
                    .check(oligo)
                    .map_err(|e| fail(format!("oligo non conforme: {e}")))?;
            }
            let decoded = decode(&encoded.oligos, &DecodeOptions::default())
                .map_err(|e| fail(format!("decode: {e}")))?;
            prop_assert!(decoded.data == data, "fichier reconstitué différent");
        }
    }
}
