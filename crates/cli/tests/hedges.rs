//! Le schéma hedges par la ligne de commande : le rendement, et les insertions et suppressions.

use std::path::PathBuf;
use std::process::{Command, Output};

const ADN: &str = env!("CARGO_BIN_EXE_adn");

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("adn-hedges-{}-{}", name, std::process::id()));
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
fn the_rate_sets_the_strand_and_the_outer_code_stays() {
    let sandbox = Sandbox::new("scheme");
    let data = pseudo_random(3_000, 5);
    let input = sandbox.write("in.bin", &data);
    // Enveloppe de 60 octets : 3 060. À ½ et à ⅓, 20 octets par brin : 153 brins de données,
    // un bloc, 32 brins de parité, dix-huit manifestes (un dixième des oligos). À ¾, 32
    // octets : 96 brins, douze manifestes. À ¼, 13 octets : 236 brins, deux blocs — le second
    // n'a que 13 brins de données, mais ses 32 de parité —, trente manifestes.
    for (name, rate, length, strands, blocks, copies) in [
        ("hedges", "1/2", 200, 153, 1, 18),
        ("indels", "3/4", 200, 96, 1, 12),
        ("hedges", "1/3", 300, 153, 1, 18),
        ("hedges", "1/4", 300, 236, 2, 30),
    ] {
        let archive = sandbox.path(&format!("archive-{length}-{rate}.fasta"));
        let report = json(&[
            "encode",
            "-i",
            &input,
            "-o",
            &archive,
            "-a",
            name,
            "--code-rate",
            rate,
        ]);
        assert_eq!(report["scheme"], "hedges", "rendement {rate}");
        assert_eq!(report["oligo_length_nt"], length, "rendement {rate}");
        assert_eq!(number(&report["units"]), strands as u64, "rendement {rate}");
        // Les brins de parité du code externe comptent parmi les oligos de l'archive : 32 par
        // bloc, et le nombre de manifestes en dépend.
        assert_eq!(
            number(&report["data_oligos"]),
            (strands + 32 * blocks) as u64,
            "rendement {rate}"
        );
        assert_eq!(
            number(&report["manifest_copies"]),
            copies,
            "rendement {rate}"
        );
        // Rien à cribler : les contraintes sont dans le code.
        assert_eq!(number(&report["candidates_tried"]), 0);
        let output = sandbox.path(&format!("out-{length}"));
        ok(&["decode", "-i", &archive, "-o", &output]);
        assert_eq!(
            &sandbox.read(&format!("out-{length}")),
            &data,
            "rendement {rate}"
        );
    }

    // Par défaut, le rendement ½.
    let archive = sandbox.path("default.fasta");
    let report = json(&["encode", "-i", &input, "-o", &archive, "-a", "hedges"]);
    assert_eq!(report["oligo_length_nt"], 200);
    let text = ok(&[
        "encode",
        "-i",
        &input,
        "-o",
        &sandbox.path("text.fasta"),
        "-a",
        "hedges",
    ]);
    assert!(
        text.contains("code HEDGES au rendement 1/2 ; 32 brins de parité par bloc de 223"),
        "{text}"
    );

    // Quatre rendements, et rien d'autre ; l'option ne sert qu'à ce schéma.
    refused(
        &[
            "encode",
            "-i",
            &input,
            "-o",
            &sandbox.path("refused.fasta"),
            "-a",
            "hedges",
            "--code-rate",
            "1/6",
        ],
        "--code-rate",
    );
    refused(
        &[
            "encode",
            "-i",
            &input,
            "-o",
            &sandbox.path("refused.fasta"),
            "-a",
            "ldpc",
            "--code-rate",
            "1/4",
        ],
        "--code-rate ne s'applique qu'au schéma hedges",
    );
    // Des contraintes plus étroites que ce que le code garantit sont refusées.
    refused(
        &[
            "encode",
            "-i",
            &input,
            "-o",
            &sandbox.path("refused.fasta"),
            "-a",
            "hedges",
            "--gc-min",
            "0.48",
        ],
        "45 % et 55 %",
    );
}

#[test]
fn hedges_survives_the_insertions_and_deletions_that_lose_the_others() {
    let sandbox = Sandbox::new("indels");
    let data = pseudo_random(1_500, 9);
    let input = sandbox.write("in.bin", &data);
    let hedges = sandbox.path("hedges.fasta");
    ok(&["encode", "-i", &input, "-o", &hedges, "-a", "hedges"]);
    let other = sandbox.path("other.fasta");
    ok(&["encode", "-i", &input, "-o", &other, "-a", "ldpc"]);

    // 0,5 % de chaque sorte d'erreur par base : restitué à chaque tirage.
    let report = json(&[
        "simulate",
        "-i",
        &hedges,
        "--archive",
        "--coverage",
        "5",
        "--substitution-rate",
        "0.005",
        "--insertion-rate",
        "0.005",
        "--deletion-rate",
        "0.005",
        "-n",
        "2",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 2, "{report}");
    assert_eq!(report["bilan"]["faux"], 0);

    // Les mêmes erreurs, sur un schéma qui ne corrige que les substitutions : rien, et jamais
    // un fichier faux.
    let report = json(&[
        "simulate",
        "-i",
        &other,
        "--archive",
        "--coverage",
        "5",
        "--substitution-rate",
        "0.005",
        "--insertion-rate",
        "0.005",
        "--deletion-rate",
        "0.005",
        "-n",
        "1",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 0, "{report}");
    assert_eq!(report["bilan"]["faux"], 0);
}

#[test]
fn the_outer_code_restores_lost_strands_up_to_a_block_in_twelve() {
    let sandbox = Sandbox::new("losses");
    // Enveloppe de 60 octets : 2 899, soit à ¼ un bloc plein — 223 brins de 13 octets, 32 de
    // parité.
    let data = pseudo_random(2_839, 11);
    let input = sandbox.write("in.bin", &data);
    let archive = sandbox.path("archive.fasta");
    ok(&[
        "encode",
        "-i",
        &input,
        "-o",
        &archive,
        "-a",
        "hedges",
        "--code-rate",
        "1/4",
    ]);

    // À couverture 4, un vingtième des brins manque en plus du tirage de Poisson : une
    // dizaine par bloc, restitués.
    let report = json(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "4",
        "--dropout",
        "0.05",
        "-n",
        "2",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 2, "{report}");
    assert_eq!(report["bilan"]["faux"], 0);

    // Un brin sur cinq : deux fois ce que les 32 brins de parité retrouvent. L'échec est annoncé.
    let report = json(&[
        "simulate",
        "-i",
        &archive,
        "--archive",
        "--coverage",
        "4",
        "--dropout",
        "0.2",
        "-n",
        "2",
    ]);
    assert_eq!(report["bilan"]["recuperes"], 0, "{report}");
    assert_eq!(report["bilan"]["faux"], 0);
}
