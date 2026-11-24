//! Tests de bout en bout : encoder, simuler la dégradation d'une archive, décoder, comparer.
//!
//! C'est la mesure utile d'un schéma de stockage : pas le taux d'erreur injecté, mais la
//! récupération du fichier. Un scénario se termine toujours par le fichier exact ou par une
//! erreur. Jamais un fichier faux.
//!
//! Ordres de grandeur mis en jeu (lectures de 152 à 160 nt) :
//! - couverture `c` (loi de Poisson) : une fraction `e^-c` des oligos n'a aucune lecture ;
//! - substitutions à `s` par nucléotide : une fraction `e^(-160 s)` des lectures est intacte ;
//! - DNA Fountain, parité de 2 octets : une erreur corrigée, deux erreurs douteuses ;
//! - brins codés : 32 brins manquants au plus par bloc de 223, soit 14 %.

use adn_core::codec::rng::SeededRng;
use adn_core::{decode, encode, DecodeOptions, DnaError, EncodeOptions, Scheme};
use adn_simulation::{ChannelConfig, DnaChannel, ErrorRates};

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SeededRng::new(seed, 0xE2E);
    let mut out = vec![0u8; len];
    rng.fill_bytes(&mut out);
    out
}

fn channel(
    sequencing: ErrorRates,
    synthesis: ErrorRates,
    dropout: f64,
    coverage: f64,
) -> ChannelConfig {
    ChannelConfig {
        synthesis,
        sequencing,
        bursts: adn_simulation::BurstConfig::NONE,
        dropout,
        coverage,
        seed: 42,
    }
}

fn sequences(reads: &[adn_simulation::SimulatedRead]) -> Vec<Vec<u8>> {
    reads.iter().map(|read| read.sequence.clone()).collect()
}

/// Encode, simule, décode. `Ok(true)` : fichier restitué ; `Ok(false)` : échec annoncé ;
/// `Err` : un fichier faux, ou un échec d'une nature inattendue.
fn run(data: &[u8], options: EncodeOptions, config: ChannelConfig) -> Result<bool, String> {
    let encoded = encode(data, &options).map_err(|e| e.to_string())?;
    let (reads, _) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    match decode(sequences(&reads), &DecodeOptions::default()) {
        Ok(decoded) if decoded.data == data => Ok(true),
        Ok(_) => Err("fichier faux".to_string()),
        Err(e) => match e {
            DnaError::InsufficientData(_)
            | DnaError::Integrity(_)
            | DnaError::UnsupportedFormat(_) => Ok(false),
            other => Err(format!("échec inattendu: {other}")),
        },
    }
}

fn fountain(redundancy: f64) -> EncodeOptions {
    EncodeOptions {
        fountain: adn_core::codec::fountain::FountainParams {
            redundancy,
            ..Default::default()
        },
        ..EncodeOptions::for_scheme(Scheme::Fountain)
    }
}

/// Taux d'erreur réalistes d'un séquenceur Illumina : substitutions, presque pas d'indels.
const ILLUMINA: ErrorRates = ErrorRates::new(0.002, 0.0, 0.0);

#[test]
fn deep_coverage_without_errors_recovers_both_schemes() {
    let data = pseudo_random(20_000, 1);
    for scheme in Scheme::ALL {
        let config = channel(ErrorRates::NONE, ErrorRates::NONE, 0.0, 30.0);
        assert!(run(&data, EncodeOptions::for_scheme(scheme), config).unwrap());
    }
}

#[test]
fn sequencing_substitutions_are_corrected_at_realistic_rates() {
    let data = pseudo_random(20_000, 2);
    for scheme in Scheme::ALL {
        for rate in [0.001, 0.005] {
            let config = channel(ErrorRates::new(rate, 0.0, 0.0), ErrorRates::NONE, 0.0, 10.0);
            assert!(
                run(&data, EncodeOptions::for_scheme(scheme), config).unwrap(),
                "{scheme:?} rate={rate}"
            );
        }
    }
}

#[test]
fn one_percent_substitution_needs_deep_coverage() {
    // À 1 %, une lecture sur cinq seulement est intacte (e^-1,6).
    let options = EncodeOptions::for_scheme(Scheme::Fountain);
    let rate = ErrorRates::new(0.01, 0.0, 0.0);
    for size in [5_000usize, 20_000] {
        let data = pseudo_random(size, 12);
        // Dix lectures par oligo : 7,5 % des oligos sans lecture intacte. Selon la marge
        // propre à l'archive, le fichier revient exact ou l'échec est annoncé.
        let config = channel(rate, ErrorRates::NONE, 0.0, 10.0);
        let _ = run(&data, options, config).unwrap();
        // Trente lectures : 0,1 % des oligos sans lecture intacte, dans la marge de 9 %.
        let config = channel(rate, ErrorRates::NONE, 0.0, 30.0);
        assert!(run(&data, options, config).unwrap(), "size={size}");
    }
}

#[test]
fn a_single_read_per_oligo_tolerates_only_untouched_reads() {
    // Une seule lecture par oligo, sans code externe : une substitution non corrigée suffit.
    // Avec un peu plus de couverture, les lectures intactes prennent le relais.
    let data = pseudo_random(5_000, 3);
    for scheme in Scheme::ALL {
        let config = channel(ILLUMINA, ErrorRates::NONE, 0.0, 1.0);
        // Couverture 1 (loi de Poisson) : plus du tiers des oligos sans lecture. L'échec est
        // certain pour le schéma Fountain (marge de 9 %) ; il est annoncé.
        let outcome = run(&data, EncodeOptions::for_scheme(scheme), config).unwrap();
        assert!(!outcome, "{scheme:?}");
    }
}

#[test]
fn more_parity_or_coverage_extends_the_tolerance_of_the_fountain() {
    let data = pseudo_random(5_000, 4);
    // 2 % de substitutions, une seule lecture par oligo en moyenne : hors de portée de la
    // parité de 2 octets (l'archive n'a pas assez de gouttes épargnées).
    let config = channel(ErrorRates::new(0.02, 0.0, 0.0), ErrorRates::NONE, 0.0, 1.0);
    assert!(!run(&data, fountain(1.1), config).unwrap());

    // Huit octets de parité corrigent quatre erreurs par lecture.
    let options = EncodeOptions {
        fountain: adn_core::codec::fountain::FountainParams {
            rs_parity: 8,
            ..Default::default()
        },
        ..EncodeOptions::for_scheme(Scheme::Fountain)
    };
    let config = channel(ErrorRates::new(0.02, 0.0, 0.0), ErrorRates::NONE, 0.0, 6.0);
    assert!(run(&data, options, config).unwrap());
}

#[test]
fn dropout_up_to_the_declared_margin_is_survived() {
    let data = pseudo_random(20_000, 5);
    // DNA Fountain : la marge demandée est tenue, et vérifiée à l'encodage.
    for seed in 0..4u64 {
        let encoded = encode(&data, &fountain(1.25)).unwrap();
        let config = ChannelConfig {
            seed,
            ..channel(ILLUMINA, ErrorRates::NONE, 0.20, 30.0)
        };
        let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
        let lost = stats.lost_oligos as f64 / f64::from(stats.oligos);
        let decoded = decode(sequences(&reads), &DecodeOptions::default())
            .unwrap_or_else(|e| panic!("perte {lost:.1} %, graine {seed} : {e}"));
        assert_eq!(decoded.data, data);
    }

    // Brins codés : 32 brins manquants au plus par bloc de 223. En perte aléatoire, le nombre
    // de brins manquants d'un bloc fluctue : 6 % laisse une marge sûre.
    let encoded = encode(&data, &EncodeOptions::for_scheme(Scheme::Strands)).unwrap();
    let config = channel(ILLUMINA, ErrorRates::NONE, 0.06, 30.0);
    let (reads, _) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    let decoded = decode(sequences(&reads), &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.data, data);

    // Au-delà de 14 % par bloc, un bloc est perdu : l'échec est annoncé, pas déguisé.
    let config = channel(ErrorRates::NONE, ErrorRates::NONE, 0.25, 30.0);
    let (reads, _) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    let err = decode(sequences(&reads), &DecodeOptions::default()).unwrap_err();
    assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");
}

#[test]
fn synthesis_errors_carried_by_every_read_are_corrected() {
    // Les erreurs de synthèse frappent le modèle : toutes les lectures d'un oligo les portent,
    // et le consensus ne peut rien y faire. Seul le code interne corrige.
    let data = pseudo_random(10_000, 6);
    let synthesis = ErrorRates::new(0.001, 0.0, 0.0);
    for scheme in Scheme::ALL {
        // Le schéma goldman n'a pas de code correcteur : une erreur de synthèse dans l'index
        // d'un brin le perd, et il ne compte que sur ses lectures. À couverture 5, un tirage
        // sur six s'en sort ; à 10, six sur sept ; à 15, tous. Les autres schémas n'ont
        // besoin que de cinq lectures par oligo.
        let coverage = if scheme == Scheme::Goldman { 20.0 } else { 5.0 };
        let config = channel(ILLUMINA, synthesis, 0.0, coverage);
        assert!(
            run(&data, EncodeOptions::for_scheme(scheme), config).unwrap(),
            "{scheme:?}"
        );
    }
}

#[test]
fn light_indels_are_absorbed_by_coverage() {
    // Une insertion ou une délétion change la longueur de la lecture, qui est perdue faute
    // d'alignement. Avec 0,2 % de chaque, une lecture sur deux garde la bonne longueur.
    let data = pseudo_random(5_000, 7);
    for scheme in Scheme::ALL {
        let config = channel(
            ErrorRates::new(0.002, 0.002, 0.002),
            ErrorRates::NONE,
            0.0,
            15.0,
        );
        assert!(
            run(&data, EncodeOptions::for_scheme(scheme), config).unwrap(),
            "{scheme:?}"
        );
    }
}

#[test]
fn heavy_indels_fail_loudly() {
    // 1 % d'insertions et 1 % de délétions : une lecture sur vingt garde sa longueur.
    let data = pseudo_random(5_000, 8);
    for scheme in Scheme::ALL {
        let config = channel(
            ErrorRates::new(0.0, 0.01, 0.01),
            ErrorRates::NONE,
            0.0,
            15.0,
        );
        let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
        let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
        assert!(stats.reads_wrong_length > stats.reads / 10);
        match decode(sequences(&reads), &DecodeOptions::default()) {
            Ok(decoded) => assert_eq!(decoded.data, data, "jamais un fichier faux"),
            Err(e) => assert!(
                matches!(e, DnaError::InsufficientData(_) | DnaError::Integrity(_)),
                "{scheme:?}: {e}"
            ),
        }
    }
}

#[test]
fn total_loss_is_reported_not_guessed() {
    let data = pseudo_random(2_000, 9);
    let encoded = encode(&data, &EncodeOptions::default()).unwrap();
    let config = channel(ErrorRates::NONE, ErrorRates::NONE, 1.0, 3.0);
    let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    assert!(reads.is_empty());
    assert_eq!(stats.lost_oligos, stats.oligos);
    let err = decode(sequences(&reads), &DecodeOptions::default()).unwrap_err();
    assert!(matches!(err, DnaError::UnsupportedFormat(_)));
}

#[test]
fn low_coverage_acts_as_dropout() {
    // Couverture 1 : environ 37 % des oligos sans lecture (e^-1). Selon la marge de l'archive,
    // le fichier revient exact ou l'échec est annoncé.
    let data = pseudo_random(2_000, 10);
    let encoded = encode(&data, &EncodeOptions::default()).unwrap();
    let config = channel(ErrorRates::NONE, ErrorRates::NONE, 0.0, 1.0);
    let (reads, stats) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    let lost = stats.lost_oligos as f64 / f64::from(stats.oligos);
    assert!((0.25..=0.50).contains(&lost), "{lost}");
    match decode(sequences(&reads), &DecodeOptions::default()) {
        Ok(decoded) => assert_eq!(decoded.data, data),
        Err(e) => assert!(matches!(e, DnaError::InsufficientData(_)), "{e}"),
    }
}

#[test]
fn simulation_is_reproducible_and_decode_is_deterministic() {
    let unit = b"Archive de donnees dans l'ADN : encoder, sequencer, decoder. ";
    let data: Vec<u8> = unit.iter().copied().cycle().take(3_000).collect();
    let encoded = encode(&data, &EncodeOptions::default()).unwrap();
    let config = ChannelConfig {
        sequencing: ErrorRates::new(0.005, 0.002, 0.003),
        dropout: 0.05,
        coverage: 6.0,
        seed: 99,
        ..ChannelConfig::default()
    };
    let (first, stats_first) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    let (second, stats_second) = DnaChannel::new(config).unwrap().simulate(&encoded.oligos);
    assert_eq!(first, second);
    assert_eq!(stats_first, stats_second);
    let mut reversed = sequences(&first);
    reversed.reverse();
    let decoded = decode(&reversed, &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.data, data);
}
