//! Benchmarks pour comparer la densité d'information des schémas d'encodage
//!
//! Mesure combien de bits sont stockés par base ADN pour chaque schéma

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use adn_core::{Encoder, EncoderConfig, EncoderType};
use std::time::Duration;

fn benchmark_information_density(c: &mut Criterion) {
    let test_data = vec![
        ("1KB_random", generate_random_data(1024)),
        ("10KB_random", generate_random_data(10 * 1024)),
        ("100KB_random", generate_random_data(100 * 1024)),
        ("1KB_repetitive", generate_repetitive_data(1024)),
        ("10KB_repetitive", generate_repetitive_data(10 * 1024)),
    ];

    let mut group = c.benchmark_group("Information Density (bits/base)");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(10);

    for (name, data) in test_data {
        let original_bits = data.len() * 8;

        // DNA Fountain (Custom)
        group.bench_function(format!("fountain_{}", name), |b| {
            let config = EncoderConfig {
                encoder_type: EncoderType::Fountain,
                chunk_size: 32,
                redundancy: 1.5,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(config).unwrap();

            b.iter(|| {
                let sequences = encoder.encode(black_box(&data)).unwrap();
                let total_bases: usize = sequences.iter().map(|s| s.bases.len()).sum();
                (original_bits, total_bases, sequences.len())
            });
        });

        // Goldman (Simple 2-bit)
        group.bench_function(format!("goldman_{}", name), |b| {
            let config = EncoderConfig {
                encoder_type: EncoderType::Goldman,
                chunk_size: 32,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(config).unwrap();

            b.iter(|| {
                let sequences = encoder.encode(black_box(&data)).unwrap();
                let total_bases: usize = sequences.iter().map(|s| s.bases.len()).sum();
                (original_bits, total_bases, sequences.len())
            });
        });

        // Goldman 2013 (with LZ4)
        group.bench_function(format!("goldman_2013_{}", name), |b| {
            let config = EncoderConfig {
                encoder_type: EncoderType::Goldman2013,
                chunk_size: 32,
                redundancy: 1.0,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(config).unwrap();

            b.iter(|| {
                let sequences = encoder.encode(black_box(&data)).unwrap();
                let total_bases: usize = sequences.iter().map(|s| s.bases.len()).sum();
                (original_bits, total_bases, sequences.len())
            });
        });

        // Erlich-Zielinski 2017 (DNA Fountain)
        group.bench_function(format!("erlich_zielinski_2017_{}", name), |b| {
            let config = EncoderConfig {
                encoder_type: EncoderType::ErlichZielinski2017,
                chunk_size: 32,
                redundancy: 1.05,
                compression_enabled: true,
                ..Default::default()
            };

            let encoder = Encoder::new(config).unwrap();

            b.iter(|| {
                let sequences = encoder.encode(black_box(&data)).unwrap();
                let total_bases: usize = sequences.iter().map(|s| s.bases.len()).sum();
                (original_bits, total_bases, sequences.len())
            });
        });

        // Grass 2015 (with Reed-Solomon)
        group.bench_function(format!("grass_2015_{}", name), |b| {
            let config = EncoderConfig {
                encoder_type: EncoderType::Grass2015,
                chunk_size: 32,
                redundancy: 1.0,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(config).unwrap();

            b.iter(|| {
                let sequences = encoder.encode(black_box(&data)).unwrap();
                let total_bases: usize = sequences.iter().map(|s| s.bases.len()).sum();
                (original_bits, total_bases, sequences.len())
            });
        });
    }

    group.finish();
}

/// Génère des données aléatoires
fn generate_random_data(size: usize) -> Vec<u8> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..size).map(|_| rng.gen()).collect()
}

/// Génère des données répétitives (pire cas pour compression)
fn generate_repetitive_data(size: usize) -> Vec<u8> {
    let pattern = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        result.extend_from_slice(pattern);
    }
    result.truncate(size);
    result
}

criterion_group!{
    name = density_benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(3));
    targets = benchmark_information_density
}

criterion_main!(density_benches);
