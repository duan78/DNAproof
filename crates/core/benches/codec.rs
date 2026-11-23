//! Mesures des codecs : encodage et décodage par schéma et par taille, avec et sans erreurs de
//! lecture, et coût des briques (compression, Reed-Solomon, contraintes).
//!
//!     cargo bench -p adn-core
//!
//! Les fichiers d'entrée sont incompressibles (octets au hasard) : c'est le cas le plus défavorable
//! pour la densité et le nombre d'oligos.

use adn_core::codec::lt::LayoutSampler;
use adn_core::codec::rng::SeededRng;
use adn_core::codec::soliton::RobustSoliton;
use adn_core::rs::RsCode;
use adn_core::{
    checksum, decode, encode, envelope, DecodeOptions, DnaConstraints, EncodeOptions, Scheme,
};
use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use std::hint::black_box;

/// Tailles mesurées, en octets.
const SIZES: [usize; 3] = [64 << 10, 256 << 10, 1 << 20];

/// Une archive abîmée : taux de substitution par nucléotide, et lectures par oligo.
type Scenario = (f64, usize);

/// Le scénario d'erreurs mesuré pour chaque schéma. Il doit rester décodable : la mesure porte
/// alors sur le décodage, pas sur un échec.
///
/// - DNA Fountain, par défaut, protège chaque oligo par deux octets de parité, qui corrigent un
///   seul octet erroné. À 0,1 % de substitution par nucléotide, 14 % des lectures de 152 nt
///   portent une erreur ; trois lectures par oligo laissent presque toujours une lecture propre.
/// - Un brin codé corrige deux octets erronés : à 0,5 % par nucléotide, 7 % des brins deviennent
///   illisibles, ce que le code externe répare (il en tolère 14 % par bloc).
fn scenario(scheme: Scheme) -> Scenario {
    match scheme {
        Scheme::Fountain => (0.001, 3),
        Scheme::Strands => (0.005, 1),
    }
}

/// Nom du scénario, pour l'intitulé de la mesure.
fn scenario_name(scheme: Scheme) -> String {
    let (rate, coverage) = scenario(scheme);
    format!(
        "{}/substitutions-{}pct-x{}",
        scheme.name(),
        format_args!("{:.1}", rate * 100.0),
        coverage
    )
}

const BASES: [u8; 4] = [b'A', b'C', b'G', b'T'];

/// Octets au hasard, reproductibles.
fn random_bytes(len: usize, stream: u64) -> Vec<u8> {
    let mut rng = SeededRng::new(0xADA, stream);
    let mut bytes = vec![0u8; len];
    rng.fill_bytes(&mut bytes);
    bytes
}

/// Une base remplacée par une autre, tirée au hasard.
fn substitute(base: u8, rng: &mut SeededRng) -> u8 {
    let rank = BASES
        .iter()
        .position(|candidate| *candidate == base)
        .unwrap_or(0);
    BASES[(rank + 1 + rng.below(3) as usize) % 4]
}

/// Lectures d'une archive : `coverage` copies, et chacune porte des substitutions au taux
/// donné (par nucléotide).
fn reads_with_errors(oligos: &[Vec<u8>], rate: f64, coverage: usize, stream: u64) -> Vec<Vec<u8>> {
    let mut rng = SeededRng::new(0xE11, stream);
    let mut reads = Vec::with_capacity(oligos.len() * coverage);
    for _ in 0..coverage {
        for oligo in oligos {
            let mut read = oligo.clone();
            for base in &mut read {
                if rng.next_f64() < rate {
                    *base = substitute(*base, &mut rng);
                }
            }
            reads.push(read);
        }
    }
    reads
}

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for scheme in Scheme::ALL {
        for size in SIZES {
            let data = random_bytes(size, size as u64);
            group.throughput(Throughput::Bytes(data.len() as u64));
            group.bench_with_input(BenchmarkId::new(scheme.name(), size), &data, |b, data| {
                b.iter(|| {
                    let options = EncodeOptions::for_scheme(scheme);
                    encode(black_box(data), &options).unwrap().oligos.len()
                })
            });
        }
    }
    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    for scheme in Scheme::ALL {
        for size in SIZES {
            let data = random_bytes(size, size as u64);
            let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
            let intact = encoded.oligos.clone();
            let (rate, coverage) = scenario(scheme);
            let damaged = reads_with_errors(&encoded.oligos, rate, coverage, size as u64);

            // La mesure n'a de sens que si le décodage réussit : une archive qui ne se décode
            // pas est refusée, jamais restituée fausse.
            let options = DecodeOptions::default();
            assert_eq!(
                decode(&intact, &options).expect("archive intacte").data,
                data
            );
            assert_eq!(
                decode(&damaged, &options).expect("archive abîmée").data,
                data
            );

            group.throughput(Throughput::Bytes(data.len() as u64));
            group.bench_with_input(
                BenchmarkId::new(format!("{}/intact", scheme.name()), size),
                &intact,
                |b, reads| b.iter(|| decode(black_box(reads), &options).unwrap().data.len()),
            );
            group.bench_with_input(
                BenchmarkId::new(scenario_name(scheme), size),
                &damaged,
                |b, reads| b.iter(|| decode(black_box(reads), &options).unwrap().data.len()),
            );
        }
    }
    group.finish();
}

/// Les briques, prises isolément.
fn bench_parts(c: &mut Criterion) {
    let mut group = c.benchmark_group("parts");

    // Contraintes biochimiques : probabilité exacte qu'un oligo au hasard passe, puis le test
    // lui-même, répété pour chaque oligo produit.
    let limits = DnaConstraints::default();
    group.bench_function("contraintes/probabilite_152nt", |b| {
        b.iter(|| black_box(limits.pass_probability(152)))
    });
    let oligo = random_dna(152);
    group.bench_function("contraintes/respect_152nt", |b| {
        b.iter(|| black_box(limits.satisfied(&oligo)))
    });

    // Reed-Solomon, sur un bloc complet de brin, et la correction de quatre octets faux.
    let code = RsCode::new(32).unwrap();
    let data = random_bytes(code.max_data_len(), 7);
    let intact = code.encode(&data).unwrap();
    let mut falsified = intact.clone();
    for position in [3, 40, 41, 200] {
        falsified[position] ^= 0x5A;
    }
    group.bench_function("reed-solomon/encoder_223+32", |b| {
        b.iter(|| black_box(code.encode(&data).unwrap().len()))
    });
    group.bench_function("reed-solomon/corriger_4_octets", |b| {
        b.iter(|| {
            let mut block = falsified.clone();
            black_box(code.correct(&mut block).unwrap())
        })
    });

    // Distribution soliton robuste et tirage d'une goutte, pour une archive de 32 768 blocs.
    let blocks = 32_768;
    let distribution = RobustSoliton::new(blocks, 0.025, 0.001).unwrap();
    group.bench_function("soliton/tirage_du_degree", |b| {
        let mut rng = SeededRng::new(1, 1);
        b.iter(|| black_box(distribution.sample(&mut rng)))
    });
    let mut sampler = LayoutSampler::new(blocks, 0.025, 0.001).unwrap();
    group.bench_function("lt/goutte_k=32768", |b| {
        let mut rng = SeededRng::new(1, 2);
        let mut indices = Vec::new();
        b.iter(|| black_box(sampler.layout(rng.next_u32(), &mut indices)))
    });

    // Empreinte et contrôle, sur un mébioctet.
    let megabyte = random_bytes(1 << 20, 9);
    group.throughput(Throughput::Bytes(megabyte.len() as u64));
    group.bench_function("sha256/1MiB", |b| {
        b.iter(|| black_box(checksum::sha256(&megabyte)))
    });
    group.bench_function("crc32/1MiB", |b| {
        b.iter(|| black_box(checksum::crc32(&megabyte)))
    });

    // Enveloppe : la compression d'octets au hasard ne rapporte rien, celle d'un texte
    // rapporte beaucoup. Les deux cas coûtent le temps de l'algorithme.
    let text = text_megabyte();
    for (name, method, input) in [
        ("aucune/hasard", envelope::Compression::None, &megabyte),
        ("zstd/hasard", envelope::Compression::Zstd, &megabyte),
        ("lz4/hasard", envelope::Compression::Lz4, &megabyte),
        ("zstd/texte", envelope::Compression::Zstd, &text),
        ("lz4/texte", envelope::Compression::Lz4, &text),
    ] {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_function(format!("enveloppe/pack_{name}"), |b| {
            b.iter(|| black_box(envelope::pack(input, method).unwrap().len()))
        });
    }
    let packed = envelope::pack(&text, envelope::Compression::Zstd).unwrap();
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("enveloppe/unpack_zstd_texte", |b| {
        b.iter(|| black_box(envelope::unpack(&packed).unwrap().len()))
    });

    group.finish();
}

/// Un oligo au hasard : qu'il respecte ou non les contraintes importe peu pour la mesure.
fn random_dna(len: usize) -> Vec<u8> {
    let mut rng = SeededRng::new(3, 3);
    (0..len).map(|_| BASES[rng.below(4) as usize]).collect()
}

/// Un mébioctet de texte compressible : des phrases tirées d'un petit vocabulaire.
fn text_megabyte() -> Vec<u8> {
    const WORDS: [&str; 32] = [
        "adn",
        "archive",
        "base",
        "code",
        "donnees",
        "deux",
        "erreur",
        "fichier",
        "fontaine",
        "goutte",
        "lectures",
        "lettre",
        "liste",
        "manip",
        "message",
        "molecule",
        "nucleotide",
        "octet",
        "oligo",
        "parite",
        "pertes",
        "reponse",
        "sequence",
        "soliton",
        "synthese",
        "taille",
        "texte",
        "tirelire",
        "tirage",
        "verbatim",
        "volume",
        "zero",
    ];
    let mut rng = SeededRng::new(5, 5);
    let mut text = String::with_capacity((1 << 20) + 64);
    while text.len() < 1 << 20 {
        for _ in 0..12 {
            text.push_str(WORDS[rng.below(32) as usize]);
            text.push(' ');
        }
        text.push_str("la fin d'une ligne.\n");
    }
    text.into_bytes()
}

criterion_group!(benches, bench_encode, bench_decode, bench_parts);
criterion_main!(benches);
