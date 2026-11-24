//! Tests d'intégration : robustesse des archives altérées.
//!
//! Propriété commune à tous ces scénarios : une archive altérée rend le fichier exact, ou une
//! erreur. Jamais un fichier faux.

use adn_core::codec::rng::SeededRng;
use adn_core::{decode, encode, DecodeOptions, DnaError, EncodeOptions, Scheme};

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SeededRng::new(seed, 0x52_4F);
    let mut out = vec![0u8; len];
    rng.fill_bytes(&mut out);
    out
}

fn flip(sequence: &mut [u8], position: usize) {
    sequence[position] = match sequence[position] {
        b'A' => b'C',
        _ => b'A',
    };
}

#[test]
fn one_substitution_per_oligo_is_corrected_in_every_scheme() {
    for scheme in Scheme::ALL {
        let data = pseudo_random(6_000, 1);
        let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
        // Hors de l'index du schéma goldman (ses quinze dernières bases, qu'une erreur rend
        // illisibles) : une substitution par lecture.
        for offset in [1usize, 20, 40, 60, 80] {
            let mut reads = encoded.oligos.clone();
            for read in &mut reads {
                let length = read.len();
                flip(read, offset % length);
            }
            let decoded = decode(&reads, &DecodeOptions::default())
                .unwrap_or_else(|error| panic!("{scheme:?} offset={offset} : {error}"));
            assert_eq!(decoded.data, data, "{scheme:?} offset={offset}");
            // Hors étiquette et nonce, toutes les lectures de données sont corrigées par le
            // code interne (les manifestes sont comptés à part). Le schéma goldman n'a pas de
            // code interne : son vote écarte l'erreur, et il n'en repère que celles qui créent
            // une répétition de bases.
            if offset >= 16 && scheme != Scheme::Goldman {
                let data_reads = reads.len() - encoded.stats.manifest_copies;
                assert!(decoded.stats.corrected > 0, "{scheme:?} offset={offset}");
                if scheme != Scheme::Grass {
                    assert_eq!(
                        decoded.stats.corrected, data_reads,
                        "{scheme:?} offset={offset}"
                    );
                }
            }
        }
    }
}

#[test]
fn n_bases_and_foreign_reads_are_ignored() {
    for scheme in Scheme::ALL {
        let data = pseudo_random(4_000, 2);
        let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
        let mut reads = encoded.oligos.clone();
        // Cinq lectures portent un N : elles sont écartées.
        for read in reads.iter_mut().take(5) {
            read[30] = b'N';
        }
        // Lectures étrangères : mauvaises longueurs, vides, caractères impossibles.
        let foreign = vec![
            b"ACGT".to_vec(),
            vec![b'G'; reads[0].len() + 3],
            Vec::new(),
            b"ACGTX".to_vec(),
        ];
        reads.extend(foreign.clone());
        let decoded = decode(&reads, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.data, data, "{scheme:?}");
        // Cinq lectures portent un N, une est vide, une a un caractère impossible.
        assert_eq!(decoded.stats.reads_invalid, 7, "{scheme:?}");
        // Une trop courte et une trop longue. Sauf en hedges : la longueur d'une lecture n'y est
        // pas un critère, une insertion ou une suppression la change. Ces deux lectures sont
        // soumises au code, qui les déclare perdues.
        let bad_length = if scheme == Scheme::Hedges { 0 } else { 2 };
        assert_eq!(decoded.stats.reads_bad_length, bad_length, "{scheme:?}");
    }
}

#[test]
fn fountain_honours_its_declared_loss_margin() {
    for redundancy in [1.1f64, 1.5, 3.0] {
        let data = pseudo_random(20_000, 3);
        let options = EncodeOptions {
            fountain: adn_core::codec::fountain::FountainParams {
                redundancy,
                ..Default::default()
            },
            ..EncodeOptions::for_scheme(Scheme::Fountain)
        };
        let encoded = encode(&data, &options).unwrap();
        let copies = encoded.stats.manifest_copies;
        // Une seule copie du manifeste suffit.
        for seed in 0..8u64 {
            let mut rng = SeededRng::new(seed, 0x1000 + seed);
            let reads: Vec<Vec<u8>> = encoded
                .oligos
                .iter()
                .enumerate()
                .filter(|(index, _)| *index < copies || rng.next_f64() < 1.0 / redundancy)
                .map(|(_, oligo)| oligo.clone())
                .collect();
            let decoded = decode(&reads, &DecodeOptions::default())
                .unwrap_or_else(|e| panic!("redundancy={redundancy} seed={seed}: {e}"));
            assert_eq!(decoded.data, data);
        }
    }
}

#[test]
fn fountain_refuses_beyond_its_loss_margin() {
    let data = pseudo_random(20_000, 4);
    let encoded = encode(&data, &EncodeOptions::default()).unwrap();
    // Marge par défaut : 9 % de perte. On retire 60 % des oligos de données.
    let copies = encoded.stats.manifest_copies;
    for seed in 0..12u64 {
        let mut rng = SeededRng::new(seed, 0x2000 + seed);
        let reads: Vec<Vec<u8>> = encoded
            .oligos
            .iter()
            .enumerate()
            .filter(|(index, _)| *index < copies || rng.next_f64() < 0.40)
            .map(|(_, oligo)| oligo.clone())
            .collect();
        match decode(&reads, &DecodeOptions::default()) {
            Ok(decoded) => assert_eq!(decoded.data, data, "jamais un fichier faux"),
            Err(e) => assert!(matches!(e, DnaError::InsufficientData(_)), "{e}"),
        }
    }
}

#[test]
fn strands_restores_missing_strands() {
    let data = pseudo_random(100_000, 5);
    let encoded = encode(&data, &EncodeOptions::for_scheme(Scheme::Strands)).unwrap();
    let copies = encoded.stats.manifest_copies;
    let mut reads = encoded.oligos.clone();
    // Trente brins consécutifs manquent : un même bloc, jusqu'à 32 restitués.
    reads.drain(copies + 10..copies + 40);
    let decoded = decode(&reads, &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.data, data);
    assert_eq!(decoded.stats.strands_repaired, 30);
    assert_eq!(decoded.stats.blocks_repaired, 1);

    // Trente-trois : le bloc est perdu, l'échec est annoncé.
    let mut reads = encoded.oligos.clone();
    reads.drain(copies + 10..copies + 43);
    let err = decode(&reads, &DecodeOptions::default()).unwrap_err();
    assert!(matches!(err, DnaError::InsufficientData(_)), "{err}");
}

#[test]
fn damaged_manifest_copies_do_not_mislead() {
    let data = pseudo_random(3_000, 6);
    let encoded = encode(&data, &EncodeOptions::default()).unwrap();
    let copies = encoded.stats.manifest_copies;
    let mut rng = SeededRng::new(9, 0x3000);

    // On abîme lourdement toutes les copies du manifeste sauf une.
    let mut reads = encoded.oligos.clone();
    for read in reads.iter_mut().take(copies).skip(1) {
        for _ in 0..12 {
            let position = rng.below(read.len() as u64) as usize;
            flip(read, position);
        }
    }
    assert_eq!(
        decode(&reads, &DecodeOptions::default()).unwrap().data,
        data
    );

    // Toutes les copies sont remplacées par des lectures aléatoires : rien n'est rendu.
    let mut reads = encoded.oligos[copies..].to_vec();
    for _ in 0..copies {
        let mut read = vec![0u8; encoded.oligos[0].len()];
        rng.fill_bytes(&mut read);
        for base in &mut read {
            *base = adn_core::sequence::BASES[(*base & 3) as usize];
        }
        reads.push(read);
    }
    let err = decode(&reads, &DecodeOptions::default()).unwrap_err();
    assert!(matches!(err, DnaError::UnsupportedFormat(_)), "{err}");
}

#[test]
fn random_tampering_never_produces_a_wrong_file() {
    for scheme in Scheme::ALL {
        let data = pseudo_random(20_000, 7);
        let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
        for trial in 0..60u64 {
            let mut rng = SeededRng::new(trial, 0x4000);
            let mut reads = encoded.oligos.clone();
            let tampered = 1 + rng.below(6);
            for _ in 0..tampered {
                let victim = rng.below(reads.len() as u64) as usize;
                let read = &mut reads[victim];
                let errors = 1 + rng.below(3);
                for _ in 0..errors {
                    let position = rng.below(read.len() as u64) as usize;
                    flip(read, position);
                }
            }
            match decode(&reads, &DecodeOptions::default()) {
                Ok(decoded) => assert_eq!(decoded.data, data, "{scheme:?} trial={trial}"),
                Err(e) => assert!(
                    matches!(
                        e,
                        DnaError::InsufficientData(_) | DnaError::UnsupportedFormat(_)
                    ),
                    "{scheme:?} trial={trial}: {e}"
                ),
            }
        }
    }
}

#[test]
fn archive_mixed_with_reads_of_another_archive_is_decoded_or_refused() {
    let first = pseudo_random(5_000, 8);
    let second = pseudo_random(5_000, 9);
    let options = EncodeOptions::default();
    let a = encode(&first, &options).unwrap();
    let b = encode(&second, &options).unwrap();

    // Un manifeste domine nettement : c'est lui qui l'emporte, l'autre archive n'est que bruit.
    let mut reads = a.oligos.clone();
    reads.extend(b.oligos[b.oligos.len() / 2..].to_vec());
    // Les lectures de b ont la même longueur et passent la parité : elles ajoutent des gouttes
    // sans rapport. Le résultat, s'il est rendu, doit être exact.
    match decode(&reads, &DecodeOptions::default()) {
        Ok(decoded) => assert_eq!(decoded.data, first),
        Err(e) => assert!(
            matches!(e, DnaError::InsufficientData(_) | DnaError::Integrity(_)),
            "{e}"
        ),
    }

    // Deux manifestes à égalité : la lecture échoue ou rend l'un des deux fichiers, exact.
    let mut reads = a.oligos.clone();
    reads.extend(b.oligos.clone());
    match decode(&reads, &DecodeOptions::default()) {
        Ok(decoded) => assert!(decoded.data == first || decoded.data == second),
        Err(e) => assert!(
            matches!(e, DnaError::InsufficientData(_) | DnaError::Integrity(_)),
            "{e}"
        ),
    }
}
