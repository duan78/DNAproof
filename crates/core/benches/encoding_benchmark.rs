//! Benchmarks pour l'encodage ADN

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use adn_core::{Encoder, EncoderConfig, EncoderType};
use std::time::Duration;

fn benchmark_encoding(c: &mut Criterion) {
    // Données de test de différentes tailles
    let test_data = vec![
        ("small", vec![0u8; 1024]),           // 1KB
        ("medium", vec![0u8; 1024 * 100]),   // 100KB
        ("large", vec![0u8; 1024 * 1024]),   // 1MB
    ];

    let mut group = c.benchmark_group("Encoding Performance");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(10);

    for (name, data) in test_data {
        group.bench_function(format!("encode_{}", name), |b| {
            let config = EncoderConfig {
                encoder_type: EncoderType::Fountain,
                chunk_size: 32,
                redundancy: 1.5,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(config).unwrap();
            
            b.iter(|| {
                let _ = encoder.encode(black_box(&data));
            });
        });
    }

    group.finish();
}

fn benchmark_fountain_vs_goldman(c: &mut Criterion) {
    let data = vec![0u8; 1024 * 100]; // 100KB

    let mut group = c.benchmark_group("Algorithm Comparison");
    group.measurement_time(Duration::from_secs(15));

    // Benchmark DNA Fountain (Custom)
    group.bench_function("fountain_encoding", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32,
            redundancy: 1.5,
            compression_enabled: false,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();

        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    // Benchmark Goldman (Simple 2-bit)
    group.bench_function("goldman_encoding", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::Goldman,
            chunk_size: 32,
            compression_enabled: false,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();

        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    // Benchmark Goldman 2013 (with LZ4)
    group.bench_function("goldman_2013_encoding", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::Goldman2013,
            chunk_size: 32,
            redundancy: 1.0,
            compression_enabled: false,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();

        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    // Benchmark Erlich-Zielinski 2017 (DNA Fountain)
    group.bench_function("erlich_zielinski_2017_encoding", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::ErlichZielinski2017,
            chunk_size: 32,
            redundancy: 1.05,
            compression_enabled: true,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();

        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    // Benchmark Grass 2015 (with Reed-Solomon)
    group.bench_function("grass_2015_encoding", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::Grass2015,
            chunk_size: 32,
            redundancy: 1.0,
            compression_enabled: false,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();

        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    group.finish();
}

fn benchmark_compression(c: &mut Criterion) {
    let data = vec![0u8; 1024 * 1000]; // 1MB

    let mut group = c.benchmark_group("Compression Performance");
    group.measurement_time(Duration::from_secs(20));

    // Sans compression
    group.bench_function("no_compression", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32,
            redundancy: 1.5,
            compression_enabled: false,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();
        
        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    // Avec compression LZ4
    group.bench_function("lz4_compression", |b| {
        let config = EncoderConfig {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32,
            redundancy: 1.5,
            compression_enabled: true,
            compression_type: adn_core::CompressionType::Lz4,
            ..Default::default()
        };

        let encoder = Encoder::new(config).unwrap();
        
        b.iter(|| {
            let _ = encoder.encode(black_box(&data));
        });
    });

    group.finish();
}

criterion_group!{
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(5));
    targets = benchmark_encoding, benchmark_fountain_vs_goldman, benchmark_compression
}

criterion_main!(benches);