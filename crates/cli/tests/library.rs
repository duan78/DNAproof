//! Bibliothèques Illumina par la ligne de commande : `adn library`, `adn simulate --library`,
//! `adn demux`, jusqu'au décodage de chaque échantillon.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ADN: &str = env!("CARGO_BIN_EXE_adn");

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("adn-library-{}-{}", name, std::process::id()));
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

    fn text(&self, name: &str) -> String {
        String::from_utf8(self.read(name)).unwrap()
    }

    fn exists(&self, name: &str) -> bool {
        Path::new(&self.path(name)).is_file()
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
    serde_json::from_str(&stdout(&output))
        .unwrap_or_else(|e| panic!("sortie JSON invalide ({e}) : {}", stdout(&output)))
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
fn refused(args: &[&str], expected: &str) -> String {
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
    message
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

/// Enregistrements d'un FASTA : en-tête et séquence.
fn records(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(header) = line.strip_prefix('>') {
            out.push((header.to_string(), String::new()));
        } else if !out.is_empty() {
            out.last_mut().unwrap().1.push_str(line.trim());
        }
    }
    out
}

/// Deux fichiers encodés (DNA Fountain et Grass), assemblés en un pool à deux index.
fn pool(sandbox: &Sandbox) -> (Vec<u8>, Vec<u8>) {
    let photo = pseudo_random(3_000, 7);
    let song = pseudo_random(3_000, 11);
    let first = sandbox.write("photo.jpg", &photo);
    let second = sandbox.write("song.mp3", &song);
    ok(&[
        "encode",
        "-i",
        &first,
        "-o",
        &sandbox.path("photo.adn.fasta"),
    ]);
    ok(&[
        "encode",
        "-i",
        &second,
        "-o",
        &sandbox.path("song.adn.fasta"),
        "-a",
        "grass",
    ]);
    ok(&[
        "library",
        "-o",
        &sandbox.path("pool.fasta"),
        "--sample",
        &format!("{}=N701+N501", sandbox.path("photo.adn.fasta")),
        "--sample",
        &format!("{}=N702+N502", sandbox.path("song.adn.fasta")),
    ]);
    (photo, song)
}

/// Séquence le pool, en gardant les lectures du tirage dans `reads/`.
fn sequence(sandbox: &Sandbox, extra: &[&str]) -> String {
    let pool = sandbox.path("pool.fasta");
    let reads = sandbox.path("reads");
    let mut args = vec![
        "simulate",
        "-i",
        &pool,
        "--library",
        "--coverage",
        "5",
        "-n",
        "1",
        "--reads",
        &reads,
    ];
    args.extend_from_slice(extra);
    ok(&args)
}

/// Arguments d'une simulation d'un pool, en couverture de 3, un tirage.
fn simulate_args<'a>(pool: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "simulate",
        "-i",
        pool,
        "--library",
        "--coverage",
        "3",
        "-n",
        "1",
    ];
    args.extend_from_slice(extra);
    args
}

/// Arguments de `adn demux` sur les lectures du sandbox. Chemins : R1 en 2, R2 en 4, I1 en 6.
fn demux_args(sandbox: &Sandbox, output: &str) -> Vec<String> {
    [
        "demux",
        "--r1",
        &sandbox.path("reads/R1.fastq"),
        "--r2",
        &sandbox.path("reads/R2.fastq"),
        "--i7",
        &sandbox.path("reads/I1.fastq"),
        "--i5",
        &sandbox.path("reads/I2.fastq"),
        "--output",
        &sandbox.path(output),
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect()
}

/// Un chemin du rapport, assemblé comme le fait le système.
fn joined(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_str().unwrap().to_string()
}

/// L'échantillon d'un rapport de démultiplexage, par son nom (ils y sont rangés par nom).
fn sample<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["echantillons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sample| sample["nom"] == name)
        .unwrap()
}

/// Un nombre du rapport JSON.
fn count(value: &serde_json::Value) -> u64 {
    value.as_u64().unwrap()
}

fn as_str(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// Décode les lectures d'un échantillon et les compare à l'original.
fn restored(sandbox: &Sandbox, reads: &str, original: &[u8]) {
    ok(&["decode", "-i", reads, "-o", &sandbox.path("restored.out")]);
    assert_eq!(&sandbox.read("restored.out"), original, "{reads}");
}

#[test]
fn a_pool_is_sequenced_demultiplexed_and_each_sample_is_restored() {
    let sandbox = Sandbox::new("pool");
    let (photo, song) = pool(&sandbox);

    // Le pool : une molécule par oligo, l'échantillon et ses index en en-tête.
    let report = json(&[
        "library",
        "-o",
        &sandbox.path("again.fasta"),
        "--sample",
        &format!("{}=N701+N501", sandbox.path("photo.adn.fasta")),
        "--sample",
        &format!("{}=N702+N502", sandbox.path("song.adn.fasta")),
    ]);
    assert_eq!(report["echantillons"].as_array().unwrap().len(), 2);
    assert_eq!(report["echantillons"][0]["nom"], "photo.adn");
    assert_eq!(report["echantillons"][0]["i7"], "N701=TAAGGCGA");
    assert_eq!(report["echantillons"][0]["i5"], "N501=TAGATCGC");
    assert_eq!(report["echantillons"][0]["insert_min"], 152);
    let again = sandbox.text("again.fasta");
    let molecules = records(&again);
    assert_eq!(report["molecules"], molecules.len() as u64);
    for (header, sequence) in &molecules {
        // P5 en tête, l'index i5, les sites d'amorce, l'extrémité mosaïque et son complément
        // inversé, P7 complément inversé en fin.
        assert!(
            header == "sample=photo.adn i7=N701:TAAGGCGA i5=N501:TAGATCGC"
                || header == "sample=song.adn i7=N702:CGTACTAG i5=N502:CTCTCTAT",
            "{header}"
        );
        assert!(
            sequence.starts_with("AATGATACGGCGACCACCGAGATCTACAC"),
            "{sequence}"
        );
        assert!(sequence.ends_with("ATCTCGTATGCCGTCTTCTGCTTG"), "{sequence}");
        assert!(
            sequence.contains("TCGTCGGCAGCGTCAGATGTGTATAAGAGACAG"),
            "{sequence}"
        );
        assert!(
            sequence.contains("CTGTCTCTTATACACATCTCCGAGCCCACGAGAC"),
            "{sequence}"
        );
    }

    // Le séquenceur : lectures 1 et 2, index, sauts d'index. Les lectures étrangères ne
    // corrompent rien : chaque échantillon est restitué à l'identique.
    let text = sequence(&sandbox, &["--index-hopping", "0.02"]);
    assert!(text.contains("1 sur 1 tirages"), "{text}");
    assert_eq!(text.matches("récupéré").count(), 2, "{text}");
    for name in ["R1", "R2", "I1", "I2"] {
        assert!(sandbox.exists(&format!("reads/{name}.fastq")), "{name}");
    }
    let r1 = sandbox.text("reads/R1.fastq");
    assert!(r1.starts_with("@SIM:1:FC:1:1101:0:0 1:N:0:"), "{r1}");
    assert!(r1.contains(" sample=photo.adn") && r1.contains(" sample=song.adn"));
    let report = json(&[
        "simulate",
        "-i",
        &sandbox.path("pool.fasta"),
        "--library",
        "--coverage",
        "5",
        "-n",
        "1",
        "--index-hopping",
        "0.02",
    ]);
    assert_eq!(report["mode"], "bibliothèque");
    assert_eq!(report["bilan"]["faux"], 0);
    assert_eq!(report["sequenceur"]["longueur_lecture"], 300);
    assert_eq!(report["sequenceur"]["sauts_index"], 0.02);
    let trial = &report["tirages"][0];
    assert!(count(&trial["sautees"]) > 0, "{report}");
    assert_eq!(trial["echantillons"][0]["nom"], "photo.adn");
    assert!(count(&trial["echantillons"][0]["fausses"]) > 0, "{report}");
    assert_eq!(trial["echantillons"][0]["recuperation"], "Exact");
    assert_eq!(trial["echantillons"][1]["recuperation"], "Exact");

    // Le démultiplexage, confronté à l'origine des lectures.
    let mut args = demux_args(&sandbox, "demux");
    args.extend([
        "--library".into(),
        sandbox.path("pool.fasta"),
        "--truth".into(),
    ]);
    let report = json(&as_str(&args));
    let truth = &report["verite"];
    assert!(count(&truth["fausses"]) > 0, "{report}");
    assert!(
        count(&truth["justes"]) > 20 * count(&truth["fausses"]),
        "{report}"
    );
    assert_eq!(truth["confusions"].as_array().unwrap().len(), 2);
    assert_eq!(
        sample(&report, "photo.adn")["fichier"],
        joined(&sandbox.path("demux"), "photo.adn.fasta")
    );
    assert_eq!(
        count(&report["paires"]),
        count(&sample(&report, "photo.adn")["paires"])
            + count(&sample(&report, "song.adn")["paires"])
            + count(&report["inconnues"])
            + count(&report["ambigues"])
    );
    let text = ok(&as_str(&args));
    assert!(text.contains("Origine des lectures"), "{text}");
    assert!(text.contains(" → "), "{text}");

    // Chaque échantillon se décode, à l'identique.
    restored(&sandbox, &sandbox.path("demux/photo.adn.fasta"), &photo);
    restored(&sandbox, &sandbox.path("demux/song.adn.fasta"), &song);
}

#[test]
fn a_library_refuses_what_an_error_would_confuse() {
    let sandbox = Sandbox::new("rules");
    let input = sandbox.write("f.bin", &pseudo_random(1_000, 3));
    ok(&["encode", "-i", &input, "-o", &sandbox.path("f.adn.fasta")]);
    let archive = sandbox.path("f.adn.fasta");
    let output = sandbox.path("pool.fasta");
    let build = |samples: &[String]| {
        let mut args: Vec<String> = vec!["library".into(), "-o".into(), output.clone()];
        for sample in samples {
            args.extend(["--sample".into(), sample.clone()]);
        }
        adn(&as_str(&args))
    };
    let spec = |name: &str, indexes: &str| format!("{name}={archive}={indexes}");
    let run = |samples: &[String]| {
        let output = build(samples);
        assert!(
            !output.status.success(),
            "la commande a réussi : {}",
            stdout(&output)
        );
        let message = format!("{}{}", stderr(&output), stdout(&output));
        assert!(!message.contains("paniqué"), "{message}");
        message
    };

    // Même index, ou trop proches : une erreur les confondrait.
    assert!(run(&[spec("a", "N701+N501"), spec("b", "N701+N502")]).contains("i7 trop proches"));
    assert!(run(&[spec("a", "N701+N501"), spec("b", "N702+TAGATCGT")]).contains("i5 trop proches"));
    // Le complément inversé d'un index.
    assert!(
        run(&[spec("a", "N701+N501"), spec("b", "TCGCCTTA+N502")]).contains("complément inversé")
    );
    // Un échantillon sur deux sans index i5.
    assert!(run(&[format!("{archive}=N701"), spec("b", "N702+N502")]).contains("index i5"));
    // Deux fois le même nom.
    assert!(run(&[spec("a", "N701+N501"), spec("a", "N702+N502")]).contains("même nom"));
    // Index inconnus.
    assert!(run(&[spec("a", "N999+N501")]).contains("nom Nextera inconnu"));
    assert!(run(&[spec("a", "N701+ACG")]).contains("séquence invalide"));

    // Sans nom, celui du fichier d'archive : accepté.
    let report = json(&[
        "library",
        "-o",
        &output,
        "--sample",
        &format!("{archive}=TAAGGCGA+TAGATCGC"),
    ]);
    assert_eq!(report["echantillons"][0]["nom"], "f.adn");
    assert_eq!(report["echantillons"][0]["i7"], "TAAGGCGA=TAAGGCGA");

    // Mal formé, absent, ou en trop.
    refused(
        &["library", "-o", &output, "--sample", &archive],
        "invalid value",
    );
    refused(&["library", "-o", &output], "required");
    refused(
        &[
            "library",
            "-o",
            &output,
            "--sample",
            &format!("{output}=N701+N501"),
        ],
        "une des archives",
    );
    // Une archive nue n'est pas un pool.
    refused(
        &[
            "simulate",
            "-i",
            &archive,
            "--library",
            "-n",
            "1",
            "--coverage",
            "5",
        ],
        "pas une bibliothèque de molécules",
    );
    // Les options du séquenceur réclament --library.
    refused(
        &["simulate", "-i", &input, "--read-length", "250"],
        "ajouter --library",
    );
    refused(
        &["simulate", "-i", &input, "--reads", &sandbox.path("r")],
        "ajouter --library",
    );
    refused(
        &[
            "simulate",
            "-i",
            &output,
            "--library",
            "--index-hopping",
            "1.5",
            "-n",
            "1",
        ],
        "sauts d'index",
    );
}

#[test]
fn demux_by_declared_indexes_and_its_refusals() {
    let sandbox = Sandbox::new("demux");
    let (photo, song) = pool(&sandbox);
    sequence(&sandbox, &[]);

    // Par les index déclarés, sans le pool : les fichiers portent le nom donné, et chacun se
    // décode.
    let mut args = demux_args(&sandbox, "declared");
    args.extend([
        "--sample".into(),
        "N701+N501=photo".into(),
        "--sample".into(),
        "N702+N502=chanson".into(),
    ]);
    let report = json(&as_str(&args));
    assert_eq!(
        sample(&report, "photo")["fichier"],
        joined(&sandbox.path("declared"), "photo.fasta")
    );
    assert!(report.get("verite").is_none(), "{report}");
    ok(&as_str(&args));
    restored(&sandbox, &sandbox.path("declared/photo.fasta"), &photo);
    restored(&sandbox, &sandbox.path("declared/chanson.fasta"), &song);

    // Sur le seul index i7, sans lecture i5.
    ok(&[
        "demux",
        "--r1",
        &sandbox.path("reads/R1.fastq"),
        "--r2",
        &sandbox.path("reads/R2.fastq"),
        "--i7",
        &sandbox.path("reads/I1.fastq"),
        "--sample",
        "N701",
        "--sample",
        "N702",
        "--output",
        &sandbox.path("single"),
    ]);
    assert!(sandbox.exists("single/N701.fasta"));
    assert!(sandbox.exists("single/N702.fasta"));

    // Les lectures peuvent être en FASTA.
    let r1 = sandbox.text("reads/R1.fastq");
    let mut fasta = String::new();
    for record in r1.lines().collect::<Vec<&str>>().chunks(4) {
        fasta.push_str(&format!(">{}\n{}\n", &record[0][1..], record[1]));
    }
    sandbox.write("reads/R1.fasta", fasta.as_bytes());
    let mut from_fasta = demux_args(&sandbox, "from_fasta");
    from_fasta[2] = sandbox.path("reads/R1.fasta");
    from_fasta.extend(["--library".into(), sandbox.path("pool.fasta")]);
    ok(&as_str(&from_fasta));
    restored(
        &sandbox,
        &sandbox.path("from_fasta/photo.adn.fasta"),
        &photo,
    );

    // Refusals.
    refused(
        &as_str(&demux_args(&sandbox, "bare")),
        "aucun échantillon connu",
    );
    let mut both = demux_args(&sandbox, "both");
    both.extend([
        "--library".into(),
        sandbox.path("pool.fasta"),
        "--sample".into(),
        "N701".into(),
    ]);
    refused(&as_str(&both), "pas les deux");
    let mut close = demux_args(&sandbox, "close");
    close.extend([
        "--sample".into(),
        "N701+N501".into(),
        "--sample".into(),
        "N701+N502".into(),
    ]);
    refused(&as_str(&close), "i7 trop proches");
    let mut name = demux_args(&sandbox, "name");
    name.extend([
        "--sample".into(),
        "N701=../parti".into(),
        "--sample".into(),
        "N702".into(),
    ]);
    refused(&as_str(&name), "nom de fichier");

    // Fichiers de longueurs différentes.
    let r2 = sandbox.text("reads/R2.fastq");
    let lines: Vec<&str> = r2.lines().collect();
    assert!(lines.len() >= 24);
    sandbox.write(
        "reads/short.fastq",
        lines[..lines.len() - 4].join("\n").as_bytes(),
    );
    let mut short = demux_args(&sandbox, "short");
    short.extend(["--library".into(), sandbox.path("pool.fasta")]);
    short[4] = sandbox.path("reads/short.fastq");
    refused(&as_str(&short), "même nombre de lectures");

    // Lectures désapparées.
    let swapped: Vec<&str> = lines[4..8]
        .iter()
        .chain(&lines[..4])
        .chain(&lines[8..])
        .copied()
        .collect();
    sandbox.write("reads/swapped.fastq", swapped.join("\n").as_bytes());
    let mut mispaired = demux_args(&sandbox, "mispaired");
    mispaired.extend(["--library".into(), sandbox.path("pool.fasta")]);
    mispaired[4] = sandbox.path("reads/swapped.fastq");
    refused(&as_str(&mispaired), "pas appariées");

    // Sans la signature du simulateur, --truth est refusé.
    let unsigned: Vec<&str> = r1
        .lines()
        .map(|line| line.split(" sample=").next().unwrap())
        .collect();
    sandbox.write("reads/unsigned.fastq", unsigned.join("\n").as_bytes());
    let mut truth = demux_args(&sandbox, "truth");
    truth[2] = sandbox.path("reads/unsigned.fastq");
    truth.extend([
        "--library".into(),
        sandbox.path("pool.fasta"),
        "--truth".into(),
    ]);
    refused(&as_str(&truth), "ne porte pas l'échantillon d'origine");
}

#[test]
fn a_read_2_that_stops_inside_the_insert_is_left_out() {
    let sandbox = Sandbox::new("partial");
    // Des brins de 300 nt sous des lectures de 300 bases : la lecture 2 part de l'extrémité
    // mosaïque, dix-neuf bases après le bout du brin, et s'arrête dix-neuf avant son début. Il
    // ne reste de chaque paire que sa lecture 1, et l'archive se décode quand même.
    let data = pseudo_random(1_000, 17);
    let input = sandbox.write("in.bin", &data);
    let archive = sandbox.path("h.adn.fasta");
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
    let pool = sandbox.path("pool.fasta");
    ok(&[
        "library",
        "-o",
        &pool,
        "--sample",
        &format!("{archive}=N701+N501"),
    ]);

    let text = ok(&simulate_args(&pool, &["--reads", &sandbox.path("reads")]));
    assert!(text.contains("1 sur 1 tirages"), "{text}");
    assert!(text.contains("lectures 2 arrêtées dans l'insert"), "{text}");
    let report = json(&simulate_args(&pool, &[]));
    let trial = &report["tirages"][0];
    assert_eq!(
        trial["echantillons"][0]["recuperation"], "Exact",
        "{report}"
    );
    assert_eq!(
        count(&trial["r2_partielles"]),
        count(&trial["paires"]) - count(&trial["inconnues"]) - count(&trial["ambigues"]),
        "{report}"
    );
    assert_eq!(
        count(&trial["echantillons"][0]["lectures"]),
        count(&trial["echantillons"][0]["paires"]),
        "{report}"
    );

    // Démultiplexées, ces lectures ne donnent que des lectures 1.
    let mut demux = demux_args(&sandbox, "demux");
    demux.extend(["--library".into(), pool.clone()]);
    let report = json(&as_str(&demux));
    assert_eq!(
        count(&report["r2_partielles"]),
        count(&sample(&report, "h.adn")["paires"]),
        "{report}"
    );
    assert_eq!(count(&report["coupees_r2"]), 0, "{report}");
    let text = ok(&as_str(&demux));
    assert!(text.contains("Lectures 2 partielles"), "{text}");
    let reads = records(&sandbox.text("demux/h.adn.fasta"));
    assert!(!reads.is_empty());
    assert!(
        reads.iter().all(|(header, _)| header.ends_with("/1")),
        "{reads:?}"
    );
    assert_eq!(
        reads.len() as u64,
        count(&sample(&report, "h.adn")["lectures"])
    );
    restored(&sandbox, &sandbox.path("demux/h.adn.fasta"), &data);

    // Trente bases de plus, et la lecture 2 rejoint le début du brin : aucune n'est écartée.
    let report = json(&simulate_args(&pool, &["--read-length", "330"]));
    let trial = &report["tirages"][0];
    assert_eq!(count(&trial["r2_partielles"]), 0, "{report}");
    assert_eq!(
        trial["echantillons"][0]["recuperation"], "Exact",
        "{report}"
    );
    assert_eq!(
        count(&trial["echantillons"][0]["lectures"]),
        2 * count(&trial["echantillons"][0]["paires"]),
        "{report}"
    );
}
