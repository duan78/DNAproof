//! Benchmark de throughput : mesure les vitesses d'encodage et décodage en MB/s.
//!
//! Usage: cargo bench -p adn-core --bench throughput_benchmark
//! Les résultats s'affichent en ns/iter ; ce fichier convertit en MB/s.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use adn_core::{Encoder, Decoder, EncoderConfig, DecoderConfig, EncoderType};
use std::time::Duration;

/// Données pseudo-aléatoires (non compressibles) pour des mesures réalistes
fn make_data(size: usize) -> Vec<u8> {
    (0..size).map(|i| ((i.wrapping_mul(2654435761usize)) as u8)).collect()
}

fn bench_encode_throughput(c: &mut Criterion) {
    let sizes: &[(usize, &str)] = &[
        (4 * 1024, "4KB"),
        (64 * 1024, "64KB"),
        (256 * 1024, "256KB"),
    ];

    let mut group = c.benchmark_group("encode_throughput");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    for &(size, label) in sizes {
        let data = make_data(size);

        // DNA Fountain
        let config = EncoderConfig {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32,
            redundancy: 2.0,
            compression_enabled: true,
            ..Default::default()
        };
        let encoder = Encoder::new(config).unwrap();

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("fountain", label), &data, |b, data| {
            b.iter(|| {
                let _ = encoder.encode(black_box(data));
            });
        });

        // Goldman 2013
        let config = EncoderConfig {
            encoder_type: EncoderType::Goldman2013,
            chunk_size: 32,
            compression_enabled: false,
            ..Default::default()
        };
        let encoder = Encoder::new(config).unwrap();

        group.bench_with_input(BenchmarkId::new("goldman2013", label), &data, |b, data| {
            b.iter(|| {
                let _ = encoder.encode(black_box(data));
            });
        });
    }

    group.finish();
}

fn bench_decode_throughput(c: &mut Criterion) {
    let sizes: &[(usize, &str)] = &[
        (4 * 1024, "4KB"),
        (64 * 1024, "64KB"),
    ];

    let mut group = c.benchmark_group("decode_throughput");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    for &(size, label) in sizes {
        let data = make_data(size);

        // Pré-encoder les données une fois
        let config = EncoderConfig {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32,
            redundancy: 2.0,
            compression_enabled: true,
            ..Default::default()
        };
        let encoder = Encoder::new(config).unwrap();
        let sequences = encoder.encode(&data).unwrap();

        let decoder = Decoder::new(DecoderConfig::default());

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("fountain_decode", label), &sequences, |b, seqs| {
            b.iter(|| {
                let _ = decoder.decode(black_box(seqs));
            });
        });
    }

    group.finish();
}

fn bench_ecc_codecs(c: &mut Criterion) {
    use adn_core::codec::reed_solomon::ReedSolomonCodec;
    use adn_core::codec::ldpc::LdpcCodec;

    let mut group = c.benchmark_group("ecc_codecs");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    // Reed-Solomon encode
    let rs = ReedSolomonCodec::new();
    let data = make_data(10 * 1024); // 10KB
    group.throughput(Throughput::Bytes(10 * 1024));
    group.bench_function("rs_encode_10KB", |b| {
        b.iter(|| {
            let _ = rs.encode(black_box(&data));
        });
    });

    // LDPC encode
    let ldpc = LdpcCodec::new(255);
    group.bench_function("ldpc_encode_10KB", |b| {
        b.iter(|| {
            let _ = ldpc.encode(black_box(&data));
        });
    });

    // LDPC decode
    let encoded = ldpc.encode(&data).unwrap();
    group.bench_function("ldpc_decode_10KB", |b| {
        b.iter(|| {
            let _ = ldpc.decode(black_box(&encoded));
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10));
    targets = bench_encode_throughput, bench_decode_throughput, bench_ecc_codecs
}

criterion_main!(benches);
