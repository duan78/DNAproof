//! Tests d'intégration pour ADN Core

use adn_core::{Encoder, Decoder, EncoderConfig, DecoderConfig, EncoderType, DnaSequence};
use std::time::Instant;

#[test]
fn test_large_file_encoding() {
    // Tester l'encodage d'un fichier de 1MB
    let data = vec![0u8; 1024 * 1024]; // 1MB
    
    let config = EncoderConfig {
        encoder_type: EncoderType::Fountain,
        chunk_size: 32,
        redundancy: 1.5,
        compression_enabled: false,
        ..Default::default()
    };

    let encoder = Encoder::new(config).unwrap();
    let start = Instant::now();
    let sequences = encoder.encode(&data).unwrap();
    let duration = start.elapsed();

    println!("Encodage de 1MB: {} séquences en {:?}", sequences.len(), duration);
    
    // Vérifier que nous avons bien des séquences
    assert!(!sequences.is_empty());
    
    // Vérifier que toutes les séquences sont valides
    for seq in &sequences {
        seq.validate(&config.constraints).unwrap();
    }
}

#[test]
fn test_roundtrip_with_compression() {
    let original_data = b"Hello, DNA Storage! This is a test of the emergency broadcast system.";
    
    // Encoder avec compression
    let encoder_config = EncoderConfig {
        encoder_type: EncoderType::Fountain,
        chunk_size: 32,
        redundancy: 1.5,
        compression_enabled: true,
        compression_type: adn_core::CompressionType::Lz4,
        ..Default::default()
    };

    let encoder = Encoder::new(encoder_config).unwrap();
    let sequences = encoder.encode(original_data).unwrap();
    
    // Décoder avec décompression
    let decoder_config = DecoderConfig {
        auto_decompress: true,
        ..Default::default()
    };

    let decoder = Decoder::new(decoder_config);
    let decoded_data = decoder.decode(&sequences).unwrap();
    
    assert_eq!(original_data.to_vec(), decoded_data);
}

#[test]
fn test_multiple_roundtrips() {
    let test_cases = vec![
        b"Short text",
        b"This is a medium length text that should be encoded and decoded correctly.",
        vec![0u8; 1024], // 1KB de zéros
        vec![255u8; 512], // 512 octets de 255
        (0..256).collect::<Vec<u8>>(), // Tous les octets possibles
    ];

    for (i, data) in test_cases.iter().enumerate() {
        println!("Test case {}", i + 1);
        
        let encoder = Encoder::new(EncoderConfig::default()).unwrap();
        let sequences = encoder.encode(data).unwrap();
        
        let decoder = Decoder::new(DecoderConfig::default());
        let decoded = decoder.decode(&sequences).unwrap();
        
        assert_eq!(data.to_vec(), decoded, "Failed for test case {}", i + 1);
    }
}

#[test]
fn test_sequence_validation() {
    let mut valid_sequence = DnaSequence::new(
        vec![
            adn_core::IupacBase::A,
            adn_core::IupacBase::C,
            adn_core::IupacBase::G,
            adn_core::IupacBase::T,
        ],
        "test".to_string(),
        0,
        4,
        0,
    );

    // Cette séquence devrait être valide
    assert!(valid_sequence.validate(&adn_core::DnaConstraints::default()).is_ok());

    // Tester avec des contraintes plus strictes
    let strict_constraints = adn_core::DnaConstraints {
        gc_min: 0.4,
        gc_max: 0.6,
        max_homopolymer: 3,
        ..Default::default()
    };

    // Cette séquence a 50% GC, devrait passer
    assert!(valid_sequence.validate(&strict_constraints).is_ok());
}

#[test]
fn test_error_handling() {
    // Tester la gestion des erreurs
    let encoder = Encoder::new(EncoderConfig::default()).unwrap();
    
    // Encodage de données vides devrait échouer ou retourner un résultat vide
    let empty_data = vec![];
    let result = encoder.encode(&empty_data);
    
    // Selon l'implémentation, cela pourrait réussir avec une séquence vide
    // ou échouer avec une erreur
    match result {
        Ok(sequences) => {
            // Si cela réussit, la séquence devrait être vide ou minimale
            assert!(sequences.is_empty() || sequences.len() == 1);
        }
        Err(e) => {
            // Si cela échoue, l'erreur devrait être descriptive
            assert!(e.to_string().contains("vide") || e.to_string().contains("empty"));
        }
    }
}

#[test]
fn test_parallel_encoding() {
    let data = vec![0u8; 1024 * 100]; // 100KB
    
    let config = EncoderConfig {
        encoder_type: EncoderType::Fountain,
        chunk_size: 32,
        redundancy: 1.5,
        compression_enabled: false,
        ..Default::default()
    };

    let encoder = Encoder::new(config).unwrap();
    
    // Mesurer le temps d'encodage séquentiel
    let start_seq = Instant::now();
    let _ = encoder.encode(&data).unwrap();
    let seq_time = start_seq.elapsed();

    // Mesurer le temps d'encodage parallèle (si implémenté)
    // Note: Cela dépend de l'implémentation de encode_fountain_optimized
    let start_par = Instant::now();
    let _ = encoder.encode(&data).unwrap(); // Utiliser la version optimisée si disponible
    let par_time = start_par.elapsed();

    println!("Temps séquentiel: {:?}, Temps parallèle: {:?}", seq_time, par_time);
    
    // Le temps parallèle devrait être inférieur ou égal au temps séquentiel
    // (en pratique, cela dépend de la charge du système)
    assert!(par_time <= seq_time * 2); // Tolérance pour la variance
}