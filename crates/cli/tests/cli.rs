//! Tests d'exécution de la ligne de commande : le binaire est lancé comme par un utilisateur.
//!
//! Chaque test travaille dans son répertoire temporaire et vérifie à la fois le code de sortie,
//! les fichiers produits et le contenu de la sortie (texte ou JSON).

use std::path::PathBuf;
use std::process::{Command, Output};

const ADN: &str = env!("CARGO_BIN_EXE_adn");

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("adn-cli-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn write(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn adn(args: &[&str]) -> Output {
    Command::new(ADN)
        .args(args)
        .output()
        .expect("lancement de adn")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// Octets pseudo-aléatoires déterministes (exclusive-orshift).
fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn text(len: usize) -> Vec<u8> {
    b"Archive de donnees dans l'ADN : encoder, sequencer, decoder. "
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

/// Taille des gros échantillons : plus modeste en débogage, où l'encodage est lent.
fn large() -> usize {
    if cfg!(debug_assertions) {
        3_000
    } else {
        60_000
    }
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(output))
        .unwrap_or_else(|e| panic!("sortie JSON invalide ({e}) : {}", stdout(output)))
}

#[test]
fn encode_then_decode_restores_the_file_for_both_schemes() {
    let sandbox = Sandbox::new("roundtrip");
    let samples: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"x".to_vec(),
        text(100),
        pseudo_random(large(), 7),
        vec![0u8; large()],
    ];
    for scheme in ["fountain", "strands"] {
        for (rank, data) in samples.iter().enumerate() {
            let input = sandbox.write(&format!("in_{rank}.bin"), data);
            let out_dir = sandbox.path(&format!("out_{scheme}_{rank}"));
            let encoded = adn(&[
                "encode",
                "-i",
                input.to_str().unwrap(),
                "-o",
                out_dir.to_str().unwrap(),
                "-a",
                scheme,
                "--json",
            ]);
            assert!(
                encoded.status.success(),
                "{scheme} n°{rank} : {}",
                stderr(&encoded)
            );
            let report = json(&encoded);
            let archive = PathBuf::from(report["output"].as_str().unwrap());

            let decoded_path = sandbox.path(&format!("dec_{scheme}_{rank}"));
            let decoded = adn(&[
                "decode",
                "-i",
                archive.to_str().unwrap(),
                "-o",
                decoded_path.to_str().unwrap(),
                "--expect-sha256",
                report["sha256"].as_str().unwrap(),
                "--json",
            ]);
            assert!(
                decoded.status.success(),
                "{scheme} n°{rank} : {}",
                stderr(&decoded)
            );
            let decoded_report = json(&decoded);
            assert_eq!(decoded_report["sha256"], report["sha256"]);
            assert_eq!(
                std::fs::read(&decoded_path).unwrap(),
                *data,
                "{scheme} n°{rank}"
            );
            assert_eq!(report["scheme"], scheme);
            assert_eq!(
                report["sha256"],
                sha256_hex(data),
                "l'empreinte annoncée est celle du fichier"
            );
        }
    }
}

/// Empreinte SHA-256, hexadécimale.
fn sha256_hex(data: &[u8]) -> String {
    adn_core::checksum::hex(&adn_core::checksum::sha256(data))
}

#[test]
fn output_flag_accepts_a_file_or_a_directory() {
    let sandbox = Sandbox::new("output");
    let input = sandbox.write("lettre.txt", b"bonjour");
    let as_file = sandbox.path("archive.fasta");
    assert!(adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        as_file.to_str().unwrap(),
        "--json"
    ])
    .status
    .success());
    assert!(as_file.is_file());
    let as_dir = sandbox.path("dossier");
    assert!(adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        as_dir.to_str().unwrap()
    ])
    .status
    .success());
    assert!(as_dir.join("lettre.fasta").is_file());
    // Sans -o : à côté de l'entrée.
    assert!(adn(&["encode", "-i", input.to_str().unwrap()])
        .status
        .success());
    assert!(sandbox.path("lettre.fasta").is_file());
    // Par défaut, un nom de fichier lisible.
    assert!(stdout(&adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        sandbox.path("x.fasta").to_str().unwrap()
    ]))
    .contains("SHA-256"));
}

#[test]
fn wrong_expected_hash_is_an_error_and_writes_nothing() {
    let sandbox = Sandbox::new("hash");
    let input = sandbox.write("f.bin", &pseudo_random(500, 3));
    let archive = sandbox.path("f.fasta");
    adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
    ]);
    let output = sandbox.path("out.bin");
    let decoded = adn(&[
        "decode",
        "-i",
        archive.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--expect-sha256",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ]);
    assert!(!decoded.status.success());
    assert!(!output.exists(), "aucun fichier ne doit être écrit");
    assert!(stderr(&decoded).contains("ne correspond pas"));

    // Empreinte mal formée.
    let decoded = adn(&[
        "decode",
        "-i",
        archive.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--expect-sha256",
        "zz",
    ]);
    assert!(!decoded.status.success());
    assert!(!output.exists());
}

#[test]
fn reads_can_be_shuffled_renamed_wrapped_and_lowercased() {
    let sandbox = Sandbox::new("shuffle");
    let data = pseudo_random(4_000, 11);
    let input = sandbox.write("f.bin", &data);
    for scheme in ["fountain", "strands"] {
        let archive = sandbox.path(&format!("{scheme}.fasta"));
        adn(&[
            "encode",
            "-i",
            input.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
            "-a",
            scheme,
        ]);
        let text_archive = std::fs::read_to_string(&archive).unwrap();
        let mut records = sequences_of(&text_archive);
        assert!(records.len() > 20, "{}", records.len());
        records.reverse();
        let mut rewritten = String::from("\n; lectures de séquençage\n");
        for (index, sequence) in records.iter().enumerate() {
            rewritten.push_str(&format!(">read_{index}\r\n"));
            for chunk in sequence.to_lowercase().as_bytes().chunks(60) {
                rewritten.push_str(&String::from_utf8_lossy(chunk));
                rewritten.push_str("\r\n");
            }
        }
        let reads = sandbox.write(&format!("{scheme}-reads.fa"), rewritten.as_bytes());
        let output = sandbox.path(&format!("{scheme}.out"));
        let decoded = adn(&[
            "decode",
            "-i",
            reads.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert!(decoded.status.success(), "{scheme} : {}", stderr(&decoded));
        assert_eq!(std::fs::read(&output).unwrap(), data, "{scheme}");
    }
}

/// Séquences d'un fichier FASTA, en-têtes retirés et lignes recollées.
fn sequences_of(fasta: &str) -> Vec<String> {
    let mut records: Vec<String> = Vec::new();
    for line in fasta.lines() {
        if line.starts_with('>') {
            records.push(String::new());
        } else if let Some(last) = records.last_mut() {
            last.push_str(line.trim());
        }
    }
    records
}

/// Archive recomposée à partir de séquences, avec des en-têtes de lecture.
fn archive_of(sequences: &[String]) -> String {
    let mut text = String::new();
    for (index, sequence) in sequences.iter().enumerate() {
        text.push_str(&format!(">read_{index}\n{sequence}\n"));
    }
    text
}

#[test]
fn damaged_archive_fails_loudly_and_writes_nothing() {
    let sandbox = Sandbox::new("damage");
    let data = pseudo_random(4_000, 13);
    let input = sandbox.write("f.bin", &data);
    let archive = sandbox.path("f.fasta");
    adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
    ]);
    let original = std::fs::read_to_string(&archive).unwrap();
    let output = sandbox.path("out.bin");

    // Un fichier tronqué de la moitié de ses séquences.
    let records = sequences_of(&original);
    let half = sandbox.write(
        "half.fasta",
        archive_of(&records[..records.len() / 2]).as_bytes(),
    );
    let decoded = adn(&[
        "decode",
        "-i",
        half.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    assert!(
        !decoded.status.success(),
        "un fichier faux ne doit jamais être rendu"
    );
    assert!(!output.exists());

    // Cinq erreurs dans chaque oligo : illisible.
    let damaged_records: Vec<String> = records
        .iter()
        .map(|sequence| {
            let mut sequence: Vec<u8> = sequence.as_bytes().to_vec();
            for position in (0..sequence.len()).step_by(29).take(5) {
                sequence[position] = match sequence[position] {
                    b'A' => b'C',
                    _ => b'A',
                };
            }
            String::from_utf8_lossy(&sequence).to_string()
        })
        .collect();
    let damaged = sandbox.write("damaged.fasta", archive_of(&damaged_records).as_bytes());
    let decoded = adn(&[
        "decode",
        "-i",
        damaged.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    assert!(!decoded.status.success());
    assert!(!output.exists());
    assert!(
        stderr(&decoded).contains("impossible") || stderr(&decoded).contains("rest"),
        "{}",
        stderr(&decoded)
    );
}

#[test]
fn retired_and_unknown_scheme_names_are_refused() {
    let sandbox = Sandbox::new("names");
    let input = sandbox.write("f.txt", "données".as_bytes());
    for name in [
        "goldman2013",
        "grass2015",
        "ultimate",
        "adaptive",
        "base3",
        "goldman",
        "choucroute",
    ] {
        let encoded = adn(&["encode", "-i", input.to_str().unwrap(), "-a", name]);
        assert!(!encoded.status.success(), "{name}");
    }
    let help = stdout(&adn(&["encode", "--help"]));
    assert!(
        help.contains("fountain") && help.contains("strands"),
        "{help}"
    );
}

#[test]
fn more_parity_lengthens_the_oligos_and_round_trips() {
    let sandbox = Sandbox::new("parity");
    let data = pseudo_random(40_000, 11);
    let input = sandbox.write("in.bin", &data);
    for parity in ["2", "8"] {
        let archive = sandbox.path(&format!("out-{parity}.fasta"));
        let encoded = adn(&[
            "encode",
            "-i",
            input.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
            "--rs-parity",
            parity,
            "--json",
        ]);
        assert!(encoded.status.success(), "{}", stderr(&encoded));
        let report = json(&encoded);
        // Graine de 4 octets, charge utile de 32, la parité : 4 nucléotides par octet.
        let expected = 4 * (4 + 32 + parity.parse::<u64>().unwrap());
        assert_eq!(report["oligo_length_nt"], expected, "parité {parity}");

        let restored = sandbox.path(&format!("dec-{parity}"));
        let decoded = adn(&[
            "decode",
            "-i",
            archive.to_str().unwrap(),
            "-o",
            restored.to_str().unwrap(),
        ]);
        assert!(decoded.status.success(), "{}", stderr(&decoded));
        assert_eq!(std::fs::read(&restored).unwrap(), data, "parité {parity}");
    }
    // Hors des clous : refusé.
    for refused in ["0", "33", "beaucoup"] {
        let archive = sandbox.path("refuse.fasta");
        let encoded = adn(&[
            "encode",
            "-i",
            input.to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
            "--rs-parity",
            refused,
        ]);
        assert!(!encoded.status.success(), "--rs-parity {refused}");
        assert!(
            !archive.exists(),
            "rien n'est écrit pour --rs-parity {refused}"
        );
    }
}

#[test]
fn legacy_archive_is_refused_with_a_hint() {
    let sandbox = Sandbox::new("legacy");
    let legacy = sandbox.write(
        "old.fasta",
        b">dc203963|scheme:erlich_zielinski_2017#5#144#l4|seed:4|chunk:32\nTTTCATCTATGTCTCGTCTACCTTCACCAGAGCCCTCGTCAGTTAGGCCAGCCAGGAAAGCCCAAGCGAACACCCGACTCACTTAGTTAGGAACGACCTCAACAACTTACCTCGGGCGTGACGTACTG\n",
    );
    let output = sandbox.path("out.bin");
    let decoded = adn(&[
        "decode",
        "-i",
        legacy.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);
    assert!(!decoded.status.success());
    assert!(!output.exists());
    assert!(
        stderr(&decoded).contains("version 0.1"),
        "{}",
        stderr(&decoded)
    );
}

#[test]
fn verify_writes_nothing_unless_asked() {
    let sandbox = Sandbox::new("verify");
    let data = pseudo_random(2_000, 17);
    let input = sandbox.write("f.bin", &data);
    let archive = sandbox.path("f.fasta");
    adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
    ]);
    let before: Vec<PathBuf> = std::fs::read_dir(&sandbox.dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();

    let verified = adn(&["verify", "-i", archive.to_str().unwrap()]);
    assert!(verified.status.success(), "{}", stderr(&verified));
    let text = stdout(&verified);
    assert!(text.contains("décodable ; empreinte vérifiée"), "{text}");
    assert!(text.contains("Aucun fichier écrit"), "{text}");
    let after: Vec<PathBuf> = std::fs::read_dir(&sandbox.dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(before, after);

    let report = json(&adn(&["verify", "-i", archive.to_str().unwrap(), "--json"]));
    assert_eq!(report["decodable"], true);
    assert_eq!(report["compliant"], report["sequences"]);
    assert_eq!(report["written"], serde_json::Value::Null);

    // Avec -o : le fichier est écrit, identique à l'original.
    let output = sandbox.path("restored.bin");
    assert!(adn(&[
        "verify",
        "-i",
        archive.to_str().unwrap(),
        "-o",
        output.to_str().unwrap()
    ])
    .status
    .success());
    assert_eq!(std::fs::read(&output).unwrap(), data);
}

#[test]
fn visualize_prints_tables_json_and_html() {
    let sandbox = Sandbox::new("visualize");
    let input = sandbox.write("f.bin", &pseudo_random(500, 19));
    let archive = sandbox.path("f.fasta");
    adn(&[
        "encode",
        "-i",
        input.to_str().unwrap(),
        "-o",
        archive.to_str().unwrap(),
    ]);
    let archive_str = archive.to_str().unwrap();
    let table = adn(&["visualize", "-i", archive_str]);
    assert!(table.status.success());
    let text = stdout(&table);
    assert!(
        text.contains("Conformes : ") && text.contains("Rang"),
        "{text}"
    );
    let report = json(&adn(&["visualize", "-i", archive_str, "-f", "json"]));
    assert_eq!(
        report["sequences"].as_array().unwrap().len(),
        report["pool"]["oligos"].as_u64().unwrap() as usize
    );
    let page = stdout(&adn(&["visualize", "-i", archive_str, "-f", "html"]));
    assert!(page.contains("<!DOCTYPE html>") && page.contains("<table>"));
    let file = sandbox.path("stats.json");
    assert!(adn(&[
        "visualize",
        "-i",
        archive_str,
        "-f",
        "json",
        "-o",
        file.to_str().unwrap()
    ])
    .status
    .success());
    serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&file).unwrap()).unwrap();
}

#[test]
fn simulate_counts_recovered_files_and_never_wrong_ones() {
    let sandbox = Sandbox::new("simulate");
    let input = sandbox.write("f.bin", &pseudo_random(2_000, 23));
    let input_str = input.to_str().unwrap();

    // Canal parfait : tout est récupéré.
    let clean = adn(&[
        "simulate",
        "-i",
        input_str,
        "--coverage",
        "20",
        "-n",
        "3",
        "--json",
    ]);
    assert!(clean.status.success(), "{}", stderr(&clean));
    let report = json(&clean);
    assert_eq!(report["bilan"]["recuperes"], 3);
    assert_eq!(report["bilan"]["faux"], 0);
    assert_eq!(report["schema"], "fountain");

    // Perte totale : tout échoue, aucun faux.
    let lost = adn(&[
        "simulate",
        "-i",
        input_str,
        "--dropout",
        "1",
        "-n",
        "2",
        "--json",
    ]);
    assert!(lost.status.success());
    let report = json(&lost);
    assert_eq!(report["bilan"]["echecs"], 2);
    assert_eq!(report["bilan"]["faux"], 0);

    // Sur une archive existante, sans le fichier d'origine.
    let archive = sandbox.path("f.fasta");
    adn(&["encode", "-i", input_str, "-o", archive.to_str().unwrap()]);
    let on_archive = adn(&[
        "simulate",
        "-i",
        archive.to_str().unwrap(),
        "--archive",
        "--coverage",
        "20",
        "--substitution-rate",
        "0.003",
        "-n",
        "4",
    ]);
    assert!(on_archive.status.success(), "{}", stderr(&on_archive));
    assert!(
        stdout(&on_archive).contains("4 sur 4 tirages"),
        "{}",
        stdout(&on_archive)
    );

    // Un taux incohérent est refusé.
    assert!(
        !adn(&["simulate", "-i", input_str, "--substitution-rate", "1.5"])
            .status
            .success()
    );
    assert!(!adn(&["simulate", "-i", input_str, "-n", "0"])
        .status
        .success());
}
