//! Tests de régression pour les bugs corrigés lors de l'audit.
//!
//! Chaque test référence le bug qu'il couvre :

use adn_core::codec::adaptive::{AdaptiveDecoder, AdaptiveEncoder};
use adn_core::codec::concatenated::ConcatenatedCodec;
use adn_core::codec::enhanced_gc_aware::EnhancedGcAwareEncoder;
use adn_core::codec::gc_aware_encoding::GcAwareEncoder;
use adn_core::codec::reed_solomon::ReedSolomonCodec;
use adn_core::codec::spreading::SpreadingCode;
use adn_core::sequence::DnaSequence;
use adn_core::{Decoder, DecoderConfig, Encoder, EncoderConfig, EncoderType};

// ---------------------------------------------------------------------------
// C1 : le round-trip de EncoderType::Adaptive via le Decoder principal était
// cassé (schéma non routé + encodage rotatif non inversé).
// ---------------------------------------------------------------------------

#[test]
fn test_adaptive_roundtrip_main_pipeline() {
    let encoder = Encoder::new(EncoderConfig {
        encoder_type: EncoderType::Adaptive,
        ..Default::default()
    })
    .unwrap();

    let original = b"Adaptive encoding round-trip via the main pipeline";
    let sequences = encoder.encode(original).unwrap();
    assert!(!sequences.is_empty());
    assert!(sequences[0]
        .metadata
        .encoding_scheme
        .starts_with("adaptive#"));

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();
    assert_eq!(original.to_vec(), recovered);
}

#[test]
fn test_adaptive_roundtrip_without_compression() {
    let encoder = Encoder::new(EncoderConfig {
        encoder_type: EncoderType::Adaptive,
        compression_enabled: false,
        ..Default::default()
    })
    .unwrap();

    let original = b"no compression adaptive roundtrip";
    let sequences = encoder.encode(original).unwrap();

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();
    assert_eq!(original.to_vec(), recovered);
}

// ---------------------------------------------------------------------------
// C2 : Ultimate choisissait Huffman/LZ4/None à l'encodage mais décompressait
// toujours en LZ4 au décodage. La méthode est maintenant taggée dans le schéma.
// ---------------------------------------------------------------------------

#[test]
fn test_ultimate_roundtrip_text_huffman() {
    // Texte → l'analyse adaptative choisit Huffman
    let encoder = Encoder::new(EncoderConfig {
        encoder_type: EncoderType::Ultimate,
        compression_enabled: false,
        ..Default::default()
    })
    .unwrap();

    let original = b"the quick brown fox jumps over the lazy dog again and again";
    let sequences = encoder.encode(original).unwrap();
    assert!(!sequences.is_empty());
    assert!(sequences[0]
        .metadata
        .encoding_scheme
        .starts_with("ultimate#"));

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();
    assert_eq!(
        original.to_vec(),
        recovered,
        "Ultimate (Huffman) round-trip"
    );
}

#[test]
fn test_ultimate_roundtrip_random_data() {
    // Données pseudo-aléatoires → méthode None ou LZ4
    let encoder = Encoder::new(EncoderConfig {
        encoder_type: EncoderType::Ultimate,
        ..Default::default()
    })
    .unwrap();

    let original: Vec<u8> = (0..600u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let sequences = encoder.encode(&original).unwrap();
    assert!(!sequences.is_empty());

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();
    assert_eq!(original, recovered, "Ultimate (random data) round-trip");
}

#[test]
fn test_ultimate_roundtrip_via_fasta() {
    // Le chunk_size (taille du payload GC-aware) doit survivre à un round-trip
    // FASTA pour que le décodeur puisse découper la section DATA.
    let encoder = Encoder::new(EncoderConfig {
        encoder_type: EncoderType::Ultimate,
        compression_enabled: false,
        ..Default::default()
    })
    .unwrap();

    let original = b"fasta roundtrip for ultimate encoded sequences";
    let sequences = encoder.encode(original).unwrap();

    let fasta: String = sequences.iter().map(|s| s.to_fasta()).collect();
    let reimported: Vec<DnaSequence> = fasta
        .split(">")
        .filter(|bloc| !bloc.trim().is_empty())
        .map(|bloc| DnaSequence::from_fasta(&format!(">{}", bloc)).unwrap())
        .collect();

    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&reimported).unwrap();
    assert_eq!(original.to_vec(), recovered);
}

// ---------------------------------------------------------------------------
// C4 : ReedSolomonCodec::is_corrupted paniquait sur des entrées de 1-3 octets.
// ---------------------------------------------------------------------------

#[test]
fn test_rs_is_corrupted_short_input_no_panic() {
    let codec = ReedSolomonCodec::new();
    for len in 0..4 {
        let data = vec![0u8; len];
        // Ne doit pas paniquer
        let _ = codec.is_corrupted(&data);
    }
}

// ---------------------------------------------------------------------------
// M1 : GcAwareEncoder tronquait silencieusement les payloads > 25 octets puis
// enregistrait la longueur non tronquée → décodage toujours en échec.
// ---------------------------------------------------------------------------

#[test]
fn test_gc_aware_rejects_oversized_payload() {
    let encoder = GcAwareEncoder::new(Default::default());
    let payload = vec![0xABu8; 26];
    let result = encoder.encode(payload, 1, 1);
    assert!(result.is_err(), "payload de 26 octets doit être rejeté");

    let mut enhanced = EnhancedGcAwareEncoder::new(Default::default());
    let result = enhanced.encode(vec![0xCDu8; 30], 1, 1);
    assert!(
        result.is_err(),
        "payload de 30 octets doit être rejeté (enhanced)"
    );
}

// ---------------------------------------------------------------------------
// M5 : AdaptiveEncoder (module adaptive) taggait ses séquences
// erlich_zielinski_2017 alors que le format est incompatible avec le
// décodeur Fountain. Il a maintenant son propre schéma + décodeur.
// ---------------------------------------------------------------------------

#[test]
fn test_adaptive_auto_roundtrip_standalone() {
    let encoder = AdaptiveEncoder::new(Default::default());
    let original = b"standalone adaptive auto roundtrip";
    let sequences = encoder.encode_auto(original).unwrap();
    assert!(!sequences.is_empty());
    assert!(sequences[0]
        .metadata
        .encoding_scheme
        .starts_with("adaptive_auto#"));

    let decoder = AdaptiveDecoder::new(Default::default());
    let scheme = sequences[0].metadata.encoding_scheme.clone();
    let recovered = decoder.decode_auto(&sequences, &scheme).unwrap();
    assert_eq!(original.to_vec(), recovered);
}

#[test]
fn test_gc_aware_scheme_tag_is_distinct() {
    // Le tag doit être distinct d'erlich_zielinski_2017 (routage incompatible)
    let encoder = GcAwareEncoder::new(Default::default());
    let seq = encoder.encode(vec![1, 2, 3], 42, 1).unwrap();
    assert_eq!(seq.metadata.encoding_scheme, "gc_aware");
}

// ---------------------------------------------------------------------------
// M3 : ConcatenatedCodec::decode retournait un octet fantôme en trop.
// ---------------------------------------------------------------------------

#[test]
fn test_concatenated_exact_roundtrip() {
    let codec = ConcatenatedCodec::new();
    for original in [
        b"A".to_vec(),
        b"Exact concatenated roundtrip!".to_vec(),
        (0..100u32).map(|i| i as u8).collect(),
    ] {
        let encoded = codec.encode(&original).unwrap();
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(
            original, decoded,
            "le round-trip concaténé doit être exact (pas d'octet fantôme)"
        );
    }
}

// ---------------------------------------------------------------------------
// M6 : EncoderConfig sans validation → panic (chunk_size=0) ou zéro séquence
// émise silencieusement (redundancy < 1).
// ---------------------------------------------------------------------------

#[test]
fn test_encoder_config_validation() {
    assert!(Encoder::new(EncoderConfig {
        chunk_size: 0,
        ..Default::default()
    })
    .is_err());

    assert!(Encoder::new(EncoderConfig {
        redundancy: 0.5,
        ..Default::default()
    })
    .is_err());

    assert!(Encoder::new(EncoderConfig {
        redundancy: f64::NAN,
        ..Default::default()
    })
    .is_err());

    assert!(Encoder::new(EncoderConfig::default()).is_ok());
}

// ---------------------------------------------------------------------------
// M8 : SpreadingCode::new paniquait sur un block_size non puissance de 2.
// L'entrelacement est exact pour toute taille >= 1.
// ---------------------------------------------------------------------------

#[test]
fn test_spreading_non_power_of_two() {
    for block_size in [1usize, 3, 5, 6, 7, 24] {
        let spreading = SpreadingCode::new(block_size);
        let data: Vec<u8> = (0..77u32).map(|i| i as u8).collect();
        let interleaved = spreading.interleave(&data);
        let recovered = spreading.deinterleave(&interleaved);
        assert_eq!(data, recovered, "block_size={}", block_size);
    }
}

// ---------------------------------------------------------------------------
// Magic bytes : les signatures de longueur != 4 n'étaient jamais détectées
// (match sur une tranche de 4 octets).
// ---------------------------------------------------------------------------

#[test]
fn test_detect_magic_bytes_variable_length() {
    use adn_core::codec::adaptive::{DataAnalyzer, DataType};

    let analyzer = DataAnalyzer::new();

    let mut jpeg = vec![0u8; 64];
    jpeg[..3].copy_from_slice(b"\xFF\xD8\xFF");
    assert_eq!(analyzer.detect_data_type(&jpeg), DataType::Image);

    let mut gzip = vec![0u8; 64];
    gzip[..2].copy_from_slice(b"\x1F\x8B");
    assert_eq!(analyzer.detect_data_type(&gzip), DataType::Compressed);

    let mut bmp = vec![0u8; 64];
    bmp[..2].copy_from_slice(b"BM");
    assert_eq!(analyzer.detect_data_type(&bmp), DataType::Image);

    let mut mp3 = vec![0u8; 64];
    mp3[..3].copy_from_slice(b"ID3");
    assert_eq!(analyzer.detect_data_type(&mp3), DataType::Audio);

    // WAV via conteneur RIFF
    let mut wav = vec![0u8; 32];
    wav[..4].copy_from_slice(b"RIFF");
    wav[8..12].copy_from_slice(b"WAVE");
    assert_eq!(analyzer.detect_data_type(&wav), DataType::Audio);
}

// ---------------------------------------------------------------------------
// C3 (déjà couvert par huffman::tests) + garde NaN métadonnées vides.
// ---------------------------------------------------------------------------

#[test]
fn test_empty_sequence_metadata_is_finite() {
    let seq = DnaSequence::new(vec![], "empty".to_string(), 0, 0, 0);
    assert!(seq.metadata.gc_ratio.is_finite());
    assert!(seq.metadata.entropy.is_finite());
}

// ---------------------------------------------------------------------------
// EZ2017 à 5 chunks : le screening GC rejette déterministement les droplets
// de degré 1 dont le chunk seul viole GC 40-60%. Sans garantie de couverture,
// certains chunks étaient indécodables (peeling bloqué). L'encodeur vérifie
// maintenant la décodabilité et complète si nécessaire.
// ---------------------------------------------------------------------------

#[test]
fn test_ez2017_multi_chunk_roundtrip() {
    // ~130 octets compressés → 5 chunks avec chunk_size 32
    let data = b"Hello DNA Storage! This is an end-to-end test file with enough content to span multiple chunks. 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ.";

    let encoder = Encoder::new(EncoderConfig {
        encoder_type: EncoderType::ErlichZielinski2017,
        chunk_size: 32,
        redundancy: 5.0,
        compression_enabled: true,
        ..Default::default()
    })
    .unwrap();

    let sequences = encoder.encode(data).unwrap();
    assert!(!sequences.is_empty());

    // Décodage direct
    let decoder = Decoder::new(DecoderConfig::default());
    let recovered = decoder.decode(&sequences).unwrap();
    assert_eq!(data.to_vec(), recovered);

    // Décodage via round-trip FASTA (comme le fait le CLI)
    let fasta: String = sequences.iter().map(|s| s.to_fasta()).collect();
    let reimported: Vec<DnaSequence> = fasta
        .split('>')
        .filter(|bloc| !bloc.trim().is_empty())
        .map(|bloc| DnaSequence::from_fasta(&format!(">{}", bloc)).unwrap())
        .collect();
    let recovered_fasta = decoder.decode(&reimported).unwrap();
    assert_eq!(data.to_vec(), recovered_fasta);
}

#[test]
fn test_fountain_peelability_guarantee_small_k() {
    // Plusieurs fichiers de tailles variées : le fountain doit toujours
    // produire un ensemble décodable, même avec peu de chunks.
    for size in [1usize, 10, 33, 64, 200, 500] {
        let data: Vec<u8> = (0..size).map(|i| (i.wrapping_mul(31) ^ i) as u8).collect();
        let encoder = Encoder::new(EncoderConfig {
            encoder_type: EncoderType::Fountain,
            chunk_size: 32,
            redundancy: 1.5,
            ..Default::default()
        })
        .unwrap();

        let sequences = encoder.encode(&data).unwrap();
        let decoder = Decoder::new(DecoderConfig::default());
        let recovered = decoder
            .decode(&sequences)
            .unwrap_or_else(|e| panic!("size={size}: {e}"));
        assert_eq!(data, recovered, "size={}", size);
    }
}
