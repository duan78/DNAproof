//! Roundtrip Validation Tests
//!
//! Tests pour vérifier que les schémas d'encodage peuvent faire un roundtrip complet

use adn_core::{Encoder, EncoderConfig, Decoder, DecoderConfig, DnaConstraints};

#[test]
fn test_roundtrip_goldman_simple() {
    let config = EncoderConfig {
        encoder_type: adn_core::codec::EncoderType::Goldman,
        chunk_size: 32,
        compression_enabled: false,
        constraints: DnaConstraints {
            gc_min: 0.20,
            gc_max: 0.80,
            max_homopolymer: 6,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        ..Default::default()
    };

    let test_data = b"Simple Goldman encoding test!";

    let encoder = Encoder::new(config).unwrap();
    let sequences = encoder.encode(test_data).unwrap();

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(test_data.to_vec(), recovered);
    println!("✅ Goldman roundtrip OK");
}

#[test]
fn test_roundtrip_goldman_all_bytes() {
    let config = EncoderConfig {
        encoder_type: adn_core::codec::EncoderType::Goldman,
        chunk_size: 32,
        compression_enabled: false,
        constraints: DnaConstraints {
            gc_min: 0.20,
            gc_max: 0.80,
            max_homopolymer: 6,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        ..Default::default()
    };

    let test_data: Vec<u8> = (0..256).map(|i| i as u8).collect();

    let encoder = Encoder::new(config).unwrap();
    let sequences = encoder.encode(&test_data).unwrap();

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(test_data, recovered);
    println!("✅ Goldman all bytes roundtrip OK");
}

#[test]
fn test_roundtrip_goldman_2013() {
    let config = EncoderConfig {
        encoder_type: adn_core::codec::EncoderType::Goldman2013,
        chunk_size: 32,
        redundancy: 1.0,
        compression_enabled: false,
        constraints: DnaConstraints {
            gc_min: 0.20,
            gc_max: 0.80,
            max_homopolymer: 6,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        ..Default::default()
    };

    let test_data = b"Goldman 2013 encoding test.";

    let encoder = Encoder::new(config).unwrap();
    let sequences = encoder.encode(test_data).unwrap();

    // Use Goldman2013 specific decoder
    let decoder = adn_core::codec::goldman_2013::Goldman2013Decoder::new(DnaConstraints::default());
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(test_data.to_vec(), recovered);
    println!("✅ Goldman 2013 roundtrip OK");
}

#[test]
fn test_roundtrip_grass_2015() {
    let config = EncoderConfig {
        encoder_type: adn_core::codec::EncoderType::Grass2015,
        chunk_size: 32,
        redundancy: 1.0,
        compression_enabled: false,
        constraints: DnaConstraints {
            gc_min: 0.0,
            gc_max: 1.0,
            max_homopolymer: 150,
            max_sequence_length: 200,
            allowed_bases: vec![
                adn_core::IupacBase::A,
                adn_core::IupacBase::C,
                adn_core::IupacBase::G,
                adn_core::IupacBase::T,
            ],
        },
        ..Default::default()
    };

    let test_data = b"Grass 2015 test with RS255 223.";

    let encoder = Encoder::new(config).unwrap();
    let sequences = encoder.encode(test_data).unwrap();

    // Use Grass2015 specific decoder
    let decoder = adn_core::codec::grass_2015::Grass2015Decoder::new(DnaConstraints::default());
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(test_data.to_vec(), recovered);
    println!("✅ Grass 2015 roundtrip OK");
}

#[test]
#[ignore = "Requires GC-aware encoder to properly handle EZ 2017 constraints (40-60% GC, <4 homopolymer)"]
fn test_roundtip_erlich_zielinski_2017() {
    let config = EncoderConfig {
        encoder_type: adn_core::codec::EncoderType::ErlichZielinski2017,
        chunk_size: 32,
        redundancy: 1.05,
        compression_enabled: false,
        constraints: DnaConstraints::new(0.40, 0.60, 3, 152),
        ..Default::default()
    };

    let test_data: Vec<u8> = (0..30).map(|i| (i * 5) as u8).collect();

    let encoder = Encoder::new(config).unwrap();
    let sequences = encoder.encode(&test_data).unwrap();

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(test_data, recovered);
    println!("✅ Erlich-Zielinski 2017 roundtrip OK");
}
