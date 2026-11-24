//! Le schéma ldpc et le canal à salves, par la ligne de commande.

use std::path::PathBuf;
use std::process::{Command, Output};

const ADN: &str = env!("CARGO_BIN_EXE_adn");

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("adn-ldpc-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_str().unwrap().to_string()
    }

    fn write(&self, name: &str, contents: &[u8]) -> String {
        let path = self.path(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.join(name)).unwrap()
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

/// Lance la commande avec `--json` et rend son rapport.
fn json(args: &[&str]) -> serde_json::Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let output = adn(&full);
    assert!(
        output.status.success(),
        "la commande a échoué : {}{}",
        stderr(&output),
        stdout(&output)
    );
    serde_json::from_str(&stdout(&output)).unwrap()
}

/// Une commande qui doit réussir : rend sa sortie.
fn ok(args: &[&str]) -> String {
    let output = adn(args);
    assert!(
        output.status.success(),
        "la commande a échoué : {}{}",
        stderr(&output),
        stdout(&output)
    );
    stdout(&output)
}

/// Une commande qui doit être refusée : rend son message, qui contient `expected`.
fn refused(args: &[&str], expected: &str) {
    let output = adn(args);
    assert!(
        !output.status.success(),
        "la commande a réussi : {}",
        stdout(&output)
    );
    let message = format!("{}{}", stderr(&output), stdout(&output));
    assert!(
        message.contains(expected),
        "attendu « {expected} » : {message}"
    );
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

fn number(value: &serde_json::Value) -> u64 {
    value.as_u64().unwrap()
}

#[test]
fn ldpc_encodes_and_decodes_and_its_parity_is_a_percentage() {
    let sandbox = Sandbox::new("scheme");
    let data = pseudo_random(3_000, 5);
    let input = sandbox.write("in.bin", &data);
    // Enveloppe de 60 octets : 3 060, soit 236 brins de 13 octets. La parité suit : 48 brins à
    // 20 %, 12 à 5 %, 118 à 50 % ; s'y ajoutent un dixième de manifestes.
    for (name, parity, strands, manifests) in [
        ("ldpc", None, 284, 28),
        ("viterbi", Some("5"), 248, 24),
        ("ldpc", Some("50"), 354, 35),
    ] {
        let archive = sandbox.path(&format!("archive-{strands}.fasta"));
        let mut args = vec!["encode", "-i", &input, "-o", &archive, "-a", name];
        if let Some(parity) = parity {
            args.extend(["--rs-parity", parity]);
        }
        let report = json(&args);
        assert_eq!(report["scheme"], "ldpc", "{name}");
        assert_eq!(report["oligo_length_nt"], 154);
        assert_eq!(number(&report["units"]), 236);
        assert_eq!(number(&report["data_oligos"]), strands);
        assert_eq!(number(&report["manifest_copies"]), manifests);
        assert!(number(&report["candidates_tried"]) >= 236);
        let output = sandbox.path(&format!("out-{strands}"));
        ok(&["decode", "-i", &archive, "-o", &output]);
        assert_eq!(&sandbox.read(&format!("out-{strands}")), &data);
    }

    // La parité est un pourcentage, de 5 à 100.
    for parity in ["0", "4", "101", "beaucoup"] {
        refused(
            &[
                "encode",
                "-i",
                &input,
                "-o",
                &sandbox.path("refused.fasta"),
                "-a",
                "ldpc",
                "--rs-parity",
                parity,
            ],
            "--rs-parity",
        );
    }
    let text = ok(&[
        "encode",
        "-i",
        &input,
        "-o",
        &sandbox.path("text.fasta"),
        "-a",
        "ldpc",
    ]);
    assert!(
        text.contains("parité LDPC de 20 % : 164 brins de parité par bloc de 819"),
        "{text}"
    );
    assert!(text.contains("de 154 nt, manifestes de 152 nt"), "{text}");
}

#[test]
fn ldpc_survives_heavy_errors_and_tells_what_it_cannot_do() {
    let sandbox = Sandbox::new("tolerance");
    let data = pseudo_random(1_500, 9);
    let input = sandbox.write("in.bin", &data);
    let archive = sandbox.path("archive.fasta");
    ok(&["encode", "-i", &input, "-o", &archive, "-a", "ldpc"]);

    // 2 % de substitution par base, cinq lectures par brin : restitué.
    let report = json(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "5",
        "--substitution-rate",
        "0.02",
        "-n",
        "3",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 3, "{report}");
    assert_eq!(report["bilan"]["faux"], 0);

    // Trop de brins sans lecture : un échec annoncé, qui dit pourquoi.
    let report = json(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "1",
        "-n",
        "3",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 0, "{report}");
    assert_eq!(report["bilan"]["faux"], 0);
}

#[test]
fn bursts_are_simulated_and_reported() {
    let sandbox = Sandbox::new("bursts");
    let data = pseudo_random(2_000, 11);
    let input = sandbox.write("in.bin", &data);
    let archive = sandbox.path("archive.fasta");
    ok(&["encode", "-i", &input, "-o", &archive, "-a", "strands"]);

    // Des salves sur 9 % des bases, longues de dix, moitié fausses : 4,5 % d'erreurs groupées.
    // Les brins codés s'en tirent à dix lectures par brin.
    let report = json(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "10",
        "--burst-rate",
        "0.01",
        "--burst-length",
        "10",
        "--burst-error",
        "0.5",
        "-n",
        "3",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 3, "{report}");
    let canal = &report["canal"];
    assert_eq!(canal["salve_taux"], 0.01);
    assert_eq!(canal["salve_longueur"], 10.0);
    assert_eq!(canal["salve_erreur"], 0.5);
    assert_eq!(canal["substitution_sequencement"], 0.001);
    // Davantage de lectures abîmées qu'avec les seules erreurs dispersées.
    let damaged: u64 = report["tirages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|trial| number(&trial["lectures"]) - number(&trial["lectures_intactes"]))
        .sum();
    assert!(damaged > 3 * 2_060 / 13 * 10 / 2, "{damaged}");

    // Sans salve, rien n'en est dit.
    let text = ok(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "10",
        "-n",
        "1",
    ]);
    assert!(!text.contains("salves"), "{text}");
    // Avec, le canal les décrit.
    let text = ok(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "10",
        "--burst-rate",
        "0.01",
        "-n",
        "1",
    ]);
    assert!(
        text.contains("salves sur 9.09 % des lectures (longueur moyenne 10 nt"),
        "{text}"
    );
    assert!(text.contains("soit 4.55 % en moyenne"), "{text}");

    // Des paramètres incohérents sont refusés.
    for (flag, value) in [
        ("--burst-rate", "2"),
        ("--burst-length", "0"),
        ("--burst-error", "1.5"),
    ] {
        refused(
            &[
                "simulate",
                "-i",
                &input,
                "--coverage",
                "5",
                flag,
                value,
                "-n",
                "1",
            ],
            "salves invalides",
        );
    }
    // Un taux négatif, donné avec un signe égal : refusé aussi.
    refused(
        &[
            "simulate",
            "-i",
            &input,
            "--coverage",
            "5",
            "--burst-rate=-1",
            "-n",
            "1",
        ],
        "salves invalides",
    );
    // Une salve entièrement fausse est admise.
    ok(&[
        "simulate",
        "-i",
        &input,
        "--coverage",
        "30",
        "--burst-rate",
        "0.001",
        "--burst-length",
        "3",
        "--burst-error",
        "1",
        "-n",
        "1",
    ]);
}
