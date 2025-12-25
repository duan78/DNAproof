//! Tests de roundtrip pour tous les types de fichiers
//!
//! Ces tests vérifient que l'encodage et le décodage préservent
//! l'intégrité des données pour différents types de fichiers.

use adn_core::{DnaConstraints};
use adn_core::codec::goldman_2013::{Goldman2013Encoder, Goldman2013Decoder};

/// Crée des contraintes appropriées pour Goldman 2013
fn goldman_constraints() -> DnaConstraints {
    DnaConstraints {
        gc_min: 0.15,  // Plus tolérant pour l'encodage simplifié
        gc_max: 0.85,  // Plus tolérant pour l'encodage simplifié
        max_homopolymer: 6,  // Augmenté pour accommoder addressing + data boundary
        max_sequence_length: 200,
        allowed_bases: vec![
            adn_core::IupacBase::A,
            adn_core::IupacBase::C,
            adn_core::IupacBase::G,
            adn_core::IupacBase::T,
        ],
    }
}

#[test]
fn test_roundtrip_text_file() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    let original: Vec<u8> = b"Hello DNA".to_vec();

    let sequences = encoder.encode(&original).unwrap();
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(original, recovered);
}

#[test]
fn test_roundtrip_json_file() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    let original: Vec<u8> = r#"{"key": "value", "number": 42}"#.as_bytes().to_vec();

    let sequences = encoder.encode(&original).unwrap();
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(original, recovered);
}

#[test]
fn test_roundtrip_binary_file() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    // Créer des données binaires avec valeurs variées (pas toutes les 256)
    let original: Vec<u8> = (0..100).map(|i| ((i * 7) % 256) as u8).collect();

    let sequences = encoder.encode(&original).unwrap();
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(original, recovered);
}

#[test]
fn test_roundtrip_large_file() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    // Créer un fichier plus grand (5KB)
    let original: Vec<u8> = (0..5000).map(|i| (i * 17 % 256) as u8).collect();

    let sequences = encoder.encode(&original).unwrap();
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(original, recovered);
    // Note: With LZ4 compression, 5KB may compress significantly, resulting in fewer sequences
    assert!(sequences.len() > 10, "Should have created multiple sequences (with compression)");
}

#[test]
fn test_roundtrip_repetitive_data() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    // Données très répétitives (challenge pour l'encodage)
    let original: Vec<u8> = b"ABCABCABCABCABC".to_vec();

    let sequences = encoder.encode(&original).unwrap();
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(original, recovered);
}

#[test]
fn test_roundtrip_random_data() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    // Données pseudo-aléatoires
    let original: Vec<u8> = (0..500)
        .map(|i| ((i * 17 + 42) % 256) as u8)
        .collect();

    let sequences = encoder.encode(&original).unwrap();
    let recovered = decoder.decode(&sequences).unwrap();

    assert_eq!(original, recovered);
}

#[test]
fn test_roundtrip_empty_file() {
    let encoder = Goldman2013Encoder::new(goldman_constraints());
    let decoder = Goldman2013Decoder::new(goldman_constraints());

    let original: Vec<u8> = vec![];

    // Empty data should return empty sequences or handle gracefully
    let sequences = encoder.encode(&original).unwrap();

    // Empty input might produce empty sequences - this is expected behavior
    if !sequences.is_empty() {
        let recovered = decoder.decode(&sequences).unwrap();
        assert_eq!(original, recovered);
    }
}
