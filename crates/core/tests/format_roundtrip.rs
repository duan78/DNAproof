//! Tests d'intégration : aller-retour complet par des fichiers FASTA, pour les deux schémas.
//!
//! Ces tests reprennent les scénarios de l'audit : en-têtes remplacés par ceux d'un séquenceur,
//! ordre des lectures mélangé, fins de ligne CRLF, minuscules, lignes vides.

use adn_core::checksum::sha256;
use adn_core::codec::rng::SeededRng;
use adn_core::{decode, encode, fasta, DecodeOptions, DnaConstraints, EncodeOptions, Scheme};

fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SeededRng::new(seed, 0x50_52);
    let mut out = vec![0u8; len];
    rng.fill_bytes(&mut out);
    out
}

/// Natures de données : aléatoire, texte, zéro, tous les octets.
fn sample(kind: usize, len: usize) -> Vec<u8> {
    match kind {
        0 => pseudo_random(len, len as u64 + 1),
        1 => {
            let words = [
                "archive",
                "données",
                "ADN",
                "oligo",
                "lectures",
                "séquence",
                "code",
                " Reed-Solomon ",
                "fountain",
                "criblage",
                "contrainte",
                "vérification",
                "SHA-256",
                "perte",
                "marge",
            ];
            let mut out = Vec::new();
            let mut rng = SeededRng::new(len as u64, 3);
            while out.len() < len {
                let word = words[rng.below(words.len() as u64) as usize];
                out.extend_from_slice(word.as_bytes());
                out.push(b' ');
            }
            out.truncate(len);
            out
        }
        2 => vec![0u8; len],
        _ => (0..len).map(|i| (i * 7 + i / 256) as u8).collect(),
    }
}

fn with_headers(oligos: &[Vec<u8>], scheme: Scheme) -> Vec<fasta::Record> {
    oligos
        .iter()
        .enumerate()
        .map(|(index, sequence)| fasta::Record {
            header: format!("adn-{}-v2 oligo={index}", scheme.name()),
            sequence: sequence.clone(),
        })
        .collect()
}

fn sequencer_headers(oligos: &[Vec<u8>]) -> Vec<fasta::Record> {
    oligos
        .iter()
        .enumerate()
        .map(|(index, sequence)| fasta::Record {
            header: format!("read_{index} length={}", sequence.len()),
            sequence: sequence.clone(),
        })
        .collect()
}

fn write_read(records: &[fasta::Record]) -> Vec<fasta::Record> {
    // Un fichier par appel : les tests tournent en parallèle dans le même processus.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let rank = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("adn_core_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("archive_{rank}.fasta"));
    fasta::write_file(&path, records).unwrap();
    let parsed = fasta::read_file(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    parsed
}

fn sequences(records: &[fasta::Record]) -> Vec<Vec<u8>> {
    records.iter().map(|r| r.sequence.clone()).collect()
}

#[test]
fn roundtrip_through_fasta_for_both_schemes_and_many_inputs() {
    let constraints = DnaConstraints::default();
    for scheme in Scheme::ALL {
        for kind in 0..4 {
            for len in [0usize, 1, 17, 31, 32, 33, 100, 1_000, 5_000] {
                let data = sample(kind, len);
                let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
                let records = write_read(&with_headers(&encoded.oligos, scheme));
                assert_eq!(records.len(), encoded.oligos.len());
                for record in &records {
                    constraints.check(&record.sequence).unwrap();
                }
                let decoded = decode(sequences(&records), &DecodeOptions::default()).unwrap();
                assert_eq!(decoded.data, data, "{scheme:?} kind={kind} len={len}");
                assert_eq!(decoded.scheme, scheme);
                assert_eq!(sha256(&decoded.data), encoded.stats.sha256);
            }
        }
    }
}

#[test]
fn decoding_uses_sequences_only() {
    for scheme in Scheme::ALL {
        let data = sample(0, 3_000);
        let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();

        // En-têtes de séquenceur, sans aucune information sur l'archive.
        let records = write_read(&sequencer_headers(&encoded.oligos));
        assert_eq!(
            decode(sequences(&records), &DecodeOptions::default())
                .unwrap()
                .data,
            data
        );

        // Ordre inverse, puis mélangé.
        let mut reversed = encoded.oligos.clone();
        reversed.reverse();
        assert_eq!(
            decode(&reversed, &DecodeOptions::default()).unwrap().data,
            data
        );
        let mut shuffled = encoded.oligos.clone();
        let mut rng = SeededRng::new(11, 12);
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, rng.below(i as u64 + 1) as usize);
        }
        assert_eq!(
            decode(&shuffled, &DecodeOptions::default()).unwrap().data,
            data
        );

        // Lectures nues en minuscules, avec des blancs et des retours à la ligne.
        let bare: Vec<String> = shuffled
            .iter()
            .map(|oligo| {
                let text = String::from_utf8(oligo.clone()).unwrap().to_lowercase();
                text.as_bytes()
                    .chunks(17)
                    .map(|chunk| String::from_utf8(chunk.to_vec()).unwrap())
                    .collect::<Vec<_>>()
                    .join(" \n")
            })
            .collect();
        assert_eq!(decode(&bare, &DecodeOptions::default()).unwrap().data, data);
    }
}

#[test]
fn fasta_tolerates_crlf_blank_lines_and_lowercase() {
    let data = sample(1, 500);
    let encoded = encode(&data, &EncodeOptions::default()).unwrap();
    let mut text = String::from("\n\n; commentaire\n");
    for (index, oligo) in encoded.oligos.iter().enumerate() {
        text.push_str(&format!(">read_{index}\r\n"));
        let lower = String::from_utf8(oligo.clone()).unwrap().to_lowercase();
        for chunk in lower.as_bytes().chunks(60) {
            text.push_str(&String::from_utf8_lossy(chunk));
            text.push_str("\r\n");
        }
        text.push('\n');
    }
    let records = fasta::read_reader(text.as_bytes()).unwrap();
    assert_eq!(records.len(), encoded.oligos.len());
    assert_eq!(
        decode(sequences(&records), &DecodeOptions::default())
            .unwrap()
            .data,
        data
    );
}

#[test]
fn stats_describe_the_archive_honestly() {
    let data = sample(0, 100_000);
    for scheme in Scheme::ALL {
        let encoded = encode(&data, &EncodeOptions::for_scheme(scheme)).unwrap();
        let stats = &encoded.stats;
        assert_eq!(stats.original_len, 100_000);
        // Données incompressibles : stockées telles quelles.
        assert_eq!(stats.body_len, 100_000 + 60);
        assert_eq!(
            stats.data_oligos + stats.manifest_copies,
            encoded.oligos.len()
        );
        let nucleotides: u64 = encoded.oligos.iter().map(|o| o.len() as u64).sum();
        assert_eq!(stats.nucleotides, nucleotides);
        let expected = 100_000.0 * 8.0 / nucleotides as f64;
        assert!((stats.bits_per_nucleotide - expected).abs() < 1e-9);
        // Densité physique : toujours inférieure à 2 bits par nucléotide.
        assert!(
            stats.bits_per_nucleotide > 0.5 && stats.bits_per_nucleotide < 2.0,
            "{scheme:?}"
        );
    }
}

#[test]
fn compression_improves_density_on_text() {
    let text = sample(1, 100_000);
    let random = sample(0, 100_000);
    let options = EncodeOptions::default();
    let text_stats = encode(&text, &options).unwrap().stats;
    let random_stats = encode(&random, &options).unwrap().stats;
    assert!(text_stats.body_len < random_stats.body_len / 2);
    assert!(text_stats.bits_per_nucleotide > random_stats.bits_per_nucleotide * 1.6);
}

#[test]
fn legacy_v1_archives_are_recognized_and_refused() {
    let legacy = ">dc203963|scheme:erlich_zielinski_2017#5#144#l4|seed:4|chunk:32\nACGTACGT\n";
    let records = fasta::read_reader(legacy.as_bytes()).unwrap();
    assert!(fasta::is_legacy_v1(&records));
    // Rien n'est rendu : pas de manifeste lisible.
    let err = decode(sequences(&records), &DecodeOptions::default()).unwrap_err();
    assert!(
        matches!(err, adn_core::DnaError::UnsupportedFormat(_)),
        "{err}"
    );
    assert!(!fasta::LEGACY_HINT.is_empty());
}

#[test]
fn strict_constraints_are_honoured_by_both_schemes() {
    // Suites d'au plus deux bases : une séquence aléatoire sur 12 500 suffit. Les deux schémas
    // trouvent leurs candidats, et rien ne sort qui ne respecte pas les contraintes.
    let constraints = DnaConstraints::new(0.45, 0.55, 2);
    let data = sample(0, 300);
    for scheme in Scheme::ALL {
        let options = EncodeOptions {
            constraints,
            ..EncodeOptions::for_scheme(scheme)
        };
        let encoded = encode(&data, &options).unwrap();
        for oligo in &encoded.oligos {
            constraints.check(oligo).unwrap();
        }
        let decoded = decode(&encoded.oligos, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.data, data, "{scheme:?}");
    }
}

#[test]
fn unsatisfiable_constraints_are_refused_up_front() {
    // Un GC d'au moins 99 % : aucune séquence de longueur raisonnable n'y parvient.
    let constraints = DnaConstraints::new(0.99, 1.0, 3);
    let started = std::time::Instant::now();
    for scheme in Scheme::ALL {
        let options = EncodeOptions {
            constraints,
            ..EncodeOptions::for_scheme(scheme)
        };
        let err = encode(b"x", &options).unwrap_err();
        assert!(matches!(err, adn_core::DnaError::Encoding(_)), "{err}");
        assert!(err.to_string().contains("trop strictes"), "{err}");
    }
    assert!(started.elapsed().as_secs() < 5);
}

#[test]
fn every_oligo_has_the_length_of_its_scheme() {
    for scheme in Scheme::ALL {
        let encoded = encode(b"x", &EncodeOptions::for_scheme(scheme)).unwrap();
        let expected = adn_core::codec::oligo_length(scheme);
        assert!(encoded.oligos.iter().all(|oligo| oligo.len() == expected));
        assert_eq!(encoded.stats.manifest_copies, 12);
    }
}

#[test]
fn more_parity_lengthens_every_oligo_manifests_included() {
    // Régression : la longueur annoncée ne tenait compte que du schéma, pas de la parité.
    for parity in [1usize, 2, 4, 8, 32] {
        let mut options = EncodeOptions::for_scheme(Scheme::Fountain);
        options.fountain.rs_parity = parity;
        let encoded = encode(b"x", &options).unwrap();
        let expected = (4 + 32 + parity) * 4;
        assert_eq!(options.oligo_length(), expected, "parité {parity}");
        assert!(
            encoded.oligos.iter().all(|oligo| oligo.len() == expected),
            "parité {parity}"
        );
        // Le manifeste porte la parité : le décodeur retrouve la longueur sans qu'on la lui donne.
        let decoded = decode(&encoded.oligos, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.data, b"x", "parité {parity}");
        assert_eq!(decoded.stats.reads_bad_length, 0, "parité {parity}");
    }
}
