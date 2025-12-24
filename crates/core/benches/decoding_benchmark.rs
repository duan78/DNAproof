//! Benchmarks pour le décodage ADN

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use adn_core::{Encoder, Decoder, EncoderConfig, DecoderConfig, EncoderType};
use std::time::Duration;

fn benchmark_decoding(c: &mut Criterion) {
    // Préparer les données d'encodage
    let original_data = vec![0u8; 1024 * 100]; // 100KB
    
    let encoder_config = EncoderConfig {
        encoder_type: EncoderType::Fountain,
        chunk_size: 32,
        redundancy: 1.5,
        compression_enabled: false,
        ..Default::default()
    };

    let encoder = Encoder::new(encoder_config).unwrap();
    let sequences = encoder.encode(&original_data).unwrap();

    let mut group = c.benchmark_group("Decoding Performance");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(10);

    group.bench_function("decode_100kb", |b| {
        let decoder = Decoder::new(DecoderConfig::default());
        
        b.iter(|| {
            let _ = decoder.decode(black_box(&sequences));
        });
    });

    group.finish();
}

fn benchmark_roundtrip(c: &mut Criterion) {
    let test_sizes = vec![
        ("1kb", vec![0u8; 1024]),
        ("10kb", vec![0u8; 1024 * 10]),
        ("100kb", vec![0u8; 1024 * 100]),
    ];

    let mut group = c.benchmark_group("Roundtrip Performance");
    group.measurement_time(Duration::from_secs(15));

    for (name, data) in test_sizes {
        group.bench_function(format!("roundtrip_{}", name), |b| {
            let encoder_config = EncoderConfig {
                encoder_type: EncoderType::Goldman, // Plus simple pour le roundtrip
                chunk_size: 32,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(encoder_config).unwrap();
            let decoder = Decoder::new(DecoderConfig::default());
            
            b.iter(|| {
                let sequences = encoder.encode(black_box(&data)).unwrap();
                let _ = decoder.decode(black_box(&sequences)).unwrap();
            });
        });
    }

    group.finish();
}

fn benchmark_fountain_decoding(c: &mut Criterion) {
    // Tester le décodage Fountain avec différents niveaux de redondance
    let original_data = vec![0u8; 1024 * 50]; // 50KB
    
    let redundancies = vec![1.0, 1.2, 1.5, 2.0];

    let mut group = c.benchmark_group("Fountain Decoding");
    group.measurement_time(Duration::from_secs(20));

    for redundancy in redundancies {
        group.bench_function(format!("fountain_redundancy_{}", redundancy), |b| {
            let encoder_config = EncoderConfig {
                encoder_type: EncoderType::Fountain,
                chunk_size: 32,
                redundancy,
                compression_enabled: false,
                ..Default::default()
            };

            let encoder = Encoder::new(encoder_config).unwrap();
            let sequences = encoder.encode(&original_data).unwrap();
            let decoder = Decoder::new(DecoderConfig::default());
            
            b.iter(|| {
                let _ = decoder.decode(black_box(&sequences)).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!{
    name = decoding_benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(5));
    targets = benchmark_decoding, benchmark_roundtrip, benchmark_fountain_decoding
}

criterion_main!(decoding_benches);