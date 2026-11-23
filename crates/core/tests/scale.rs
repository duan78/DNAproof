//! Tests d'échelle : temps et mémoire sur de grosses entrées.
//!
//! Ignorés par défaut (plusieurs secondes chacun en mode débogage). Les lancer avec
//! `cargo test -p adn-core --release --test scale -- --ignored --nocapture`.

use adn_core::codec::rng::SeededRng;
use adn_core::{decode, encode, DecodeOptions, EncodeOptions, Scheme};
use std::time::Instant;

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SeededRng::new(seed, 0x0005_CA1E);
    let mut out = vec![0u8; len];
    rng.fill_bytes(&mut out);
    out
}

fn megabytes(count: usize) -> Vec<u8> {
    pseudo_random(count * 1_048_576, count as u64)
}

fn roundtrip(scheme: Scheme, data: &[u8]) {
    let started = Instant::now();
    let encoded = encode(data, &EncodeOptions::for_scheme(scheme)).unwrap();
    let encode_time = started.elapsed();
    let started = Instant::now();
    let decoded = decode(&encoded.oligos, &DecodeOptions::default()).unwrap();
    let decode_time = started.elapsed();
    assert_eq!(decoded.data, data);
    let seconds = |d: std::time::Duration| d.as_secs_f64();
    println!(
        "SCALE scheme={} bytes={} oligos={} bits/nt={:.3} encode={:.2}s ({:.1} MB/s) decode={:.2}s ({:.1} MB/s)",
        scheme.name(),
        data.len(),
        encoded.oligos.len(),
        encoded.stats.bits_per_nucleotide,
        seconds(encode_time),
        data.len() as f64 / 1_048_576.0 / seconds(encode_time),
        seconds(decode_time),
        data.len() as f64 / 1_048_576.0 / seconds(decode_time),
    );
}

#[test]
#[ignore = "test d'échelle : lent en débogage"]
fn one_megabyte_in_both_schemes() {
    let data = megabytes(1);
    for scheme in Scheme::ALL {
        roundtrip(scheme, &data);
    }
}

#[test]
#[ignore = "test d'échelle : lent en débogage"]
fn eight_megabytes_in_both_schemes() {
    let data = megabytes(8);
    for scheme in Scheme::ALL {
        roundtrip(scheme, &data);
    }
}

#[test]
#[ignore = "test d'échelle : lent en débogage"]
fn encoding_time_grows_less_than_quadratically() {
    // Trois tailles, chaque niveau décuple le précédent en environ. Entre deux niveaux, le
    // rapport des temps doit rester inférieur au carré du rapport des tailles.
    let mut timings = Vec::new();
    for count in [100_000usize, 1_000_000, 10_000_000] {
        let data = pseudo_random(count, count as u64);
        let started = Instant::now();
        let encoded = encode(&data, &EncodeOptions::default()).unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        let decoded = decode(&encoded.oligos, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.data, data);
        println!(
            "SCALE growth bytes={count} encode={elapsed:.2}s bits/nt={:.3}",
            encoded.stats.bits_per_nucleotide
        );
        timings.push((count as f64, elapsed));
    }
    for pair in timings.windows(2) {
        let (small_size, small_time) = pair[0];
        let (big_size, big_time) = pair[1];
        let size_ratio = big_size / small_size;
        let time_ratio = big_time / small_time;
        assert!(
            time_ratio < size_ratio * size_ratio,
            "taille x{size_ratio:.1}, temps x{time_ratio:.1}"
        );
    }
}

#[test]
#[ignore = "test d'échelle : lent en débogage"]
fn compressible_hundred_megabytes() {
    // Texte très redondant : la compression ramène le corps à quelques méga-octets.
    let unit = b"Le renard brun saute par-dessus le chien paresseux. ";
    let data: Vec<u8> = unit.iter().copied().cycle().take(100 * 1_048_576).collect();
    roundtrip(Scheme::Fountain, &data);
}
