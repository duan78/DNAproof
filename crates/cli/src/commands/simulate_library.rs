//! `adn simulate --library` : séquence un pool, démultiplexe, puis décode chaque échantillon.
//!
//! Le pool vient d'`adn library`. Chaque tirage passe par le modèle de séquenceur (lectures
//! appariées, lectures d'index, sauts d'index), le démultiplexage et la coupe des adaptateurs,
//! puis le décodage de chaque échantillon. Le bilan compte les échantillons restitués à
//! l'identique. Un échec est toujours annoncé : un fichier qui ne porte pas l'empreinte de
//! l'archive n'est jamais rendu.

use crate::commands::demux::{demultiplex, RawPair};
use crate::commands::simulate::{Bilan, Recovered};
use crate::output::{seconds, Fields, Printer};
use adn_core::fasta::FastqWriter;
use adn_core::library::{read_pool, Library, PoolMolecule};
use adn_core::{decode, DecodeOptions};
use adn_simulation::{read_header, read_id, ErrorRates, ReadPair, Sequencer, SequencerConfig};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use std::time::Instant;

#[derive(Serialize)]
struct Trial {
    tirage: usize,
    paires: usize,
    molecules_perdues: usize,
    /// Paires aux index empruntés à une autre molécule.
    sautees: usize,
    ambigues: usize,
    inconnues: usize,
    echantillons: Vec<TrialSample>,
}

#[derive(Serialize)]
struct TrialSample {
    nom: String,
    paires: usize,
    lectures: usize,
    /// Pairs rangées ici qui venaient d'un autre échantillon.
    fausses: usize,
    recuperation: Recovered,
}

#[derive(Serialize)]
struct SequencerSettings {
    longueur_lecture: usize,
    longueur_lecture_index: usize,
    substitution_sequencement: f64,
    insertion_sequencement: f64,
    deletion_sequencement: f64,
    substitution_index: f64,
    substitution_synthese: f64,
    insertion_synthese: f64,
    deletion_synthese: f64,
    perte: f64,
    couverture: f64,
    sauts_index: f64,
}

#[derive(Serialize)]
struct Report {
    entree: String,
    mode: &'static str,
    sequenceur: SequencerSettings,
    tirages: Vec<Trial>,
    bilan: Bilan,
}

pub(crate) fn run(
    spec: &crate::RunSpec<'_>,
    channel: &crate::ChannelArgs,
    sequencer: &crate::SequencerArgs,
    out: &Printer,
) -> Result<()> {
    let crate::RunSpec {
        input,
        trials,
        seed,
    } = *spec;
    if trials == 0 {
        bail!("--trials doit être au moins 1");
    }
    let records = crate::commands::read_reads(&[input.to_path_buf()])?;
    let (library, molecules) = read_pool(&records).with_context(|| {
        format!(
            "{} n'est pas une bibliothèque de molécules (sortie d'`adn library`)",
            input.display()
        )
    })?;
    let config = SequencerConfig {
        read_length: sequencer.read_length,
        index_read_length: sequencer.index_read_length,
        sequencing: ErrorRates::new(
            channel.substitution_rate,
            channel.insertion_rate,
            channel.deletion_rate,
        ),
        index_sequencing: ErrorRates::new(sequencer.index_substitution_rate, 0.0, 0.0),
        synthesis: ErrorRates::new(
            channel.synthesis_substitution_rate,
            channel.synthesis_insertion_rate,
            channel.synthesis_deletion_rate,
        ),
        dropout: channel.dropout,
        coverage: channel.coverage,
        index_hopping: sequencer.index_hopping,
        seed,
    };
    let index_read_length = Sequencer::new(config)
        .context("paramètres du séquenceur incohérents")?
        .index_read_length(&library);

    let started = Instant::now();
    let mut results = Vec::with_capacity(trials);
    // Empreinte du fichier restitué de chaque échantillon, au premier tirage réussi.
    let mut fingerprints: Vec<Option<[u8; 32]>> = vec![None; library.samples().len()];
    for trial in 0..trials {
        let config = SequencerConfig {
            seed: seed.wrapping_add(trial as u64),
            ..config
        };
        let (pairs, stats) = Sequencer::new(config)?.run(&library, &molecules)?;
        let ids: Vec<String> = (0..pairs.len()).map(read_id).collect();
        let origins: Vec<&str> = pairs
            .iter()
            .map(|pair| {
                library
                    .sample(molecules[pair.molecule as usize].sample)
                    .name
                    .as_str()
            })
            .collect();
        let raw: Vec<RawPair<'_>> = pairs
            .iter()
            .enumerate()
            .map(|(rank, pair)| RawPair {
                id: &ids[rank],
                r1: &pair.r1,
                r2: &pair.r2,
                i7: &pair.i7,
                i5: pair.i5.as_deref(),
                origin: Some(origins[rank]),
            })
            .collect();
        let mut reads: Vec<Vec<Vec<u8>>> = vec![Vec::new(); library.samples().len()];
        let demux = demultiplex(&library, &raw, &mut |sample, _, sequence| {
            reads[sample].push(sequence.to_vec());
            Ok(())
        })?;
        if trial == 0 {
            if let Some(dir) = &sequencer.reads {
                write_reads(dir, &library, &molecules, &pairs, &ids)?;
            }
        }
        let mut samples = Vec::with_capacity(demux.echantillons.len());
        for (rank, counts) in demux.echantillons.iter().enumerate() {
            let recuperation = match decode(&reads[rank], &DecodeOptions::default()) {
                Ok(decoded) => {
                    // Le fichier rendu porte l'empreinte de l'archive. D'un tirage à l'autre,
                    // un même échantillon doit rendre le même fichier.
                    if let Some(seen) = &fingerprints[rank] {
                        if *seen != decoded.sha256 {
                            bail!(
                                "tirage {} : deux tirages réussis ont rendu des fichiers \
                                 différents pour l'échantillon {}",
                                trial + 1,
                                counts.nom
                            );
                        }
                    } else {
                        fingerprints[rank] = Some(decoded.sha256);
                    }
                    Recovered::Exact
                }
                Err(_) => Recovered::Echec,
            };
            let fausses = demux.verite.as_ref().map_or(0, |truth| {
                truth
                    .confusions
                    .iter()
                    .filter(|confusion| confusion.range == counts.nom)
                    .map(|confusion| confusion.lectures)
                    .sum::<usize>()
            });
            samples.push(TrialSample {
                nom: counts.nom.clone(),
                paires: counts.paires,
                lectures: counts.lectures,
                fausses,
                recuperation,
            });
        }
        results.push(Trial {
            tirage: trial + 1,
            paires: stats.pairs,
            molecules_perdues: stats.lost_molecules,
            sautees: stats.hopped,
            ambigues: demux.ambigues,
            inconnues: demux.inconnues,
            echantillons: samples,
        });
    }
    let elapsed = started.elapsed().as_secs_f64();
    let recovered = results
        .iter()
        .filter(|trial| {
            trial
                .echantillons
                .iter()
                .all(|sample| matches!(sample.recuperation, Recovered::Exact))
        })
        .count();
    let report = Report {
        entree: input.to_string_lossy().to_string(),
        mode: "bibliothèque",
        sequenceur: SequencerSettings {
            longueur_lecture: config.read_length,
            longueur_lecture_index: index_read_length,
            substitution_sequencement: config.sequencing.substitution,
            insertion_sequencement: config.sequencing.insertion,
            deletion_sequencement: config.sequencing.deletion,
            substitution_index: config.index_sequencing.substitution,
            substitution_synthese: config.synthesis.substitution,
            insertion_synthese: config.synthesis.insertion,
            deletion_synthese: config.synthesis.deletion,
            perte: config.dropout,
            couverture: config.coverage,
            sauts_index: config.index_hopping,
        },
        tirages: results,
        bilan: Bilan {
            tirages: trials,
            recuperes: recovered,
            echecs: trials - recovered,
            faux: 0,
            duree_totale_s: elapsed,
        },
    };

    let percent = |rate: f64| format!("{:.2} %", rate * 100.0);
    out.finish(&report, || {
        let mut text = format!(
            "Simulation d'un séquenceur sur {} : {} tirages, {} échantillons, {} molécules\n",
            crate::commands::name(input),
            trials,
            library.samples().len(),
            crate::output::thousands(molecules.len() as u64)
        );
        text.push_str(&format!(
            "Lectures de {} bases, index de {} ; couverture {} ; séquençage {} / {} / {}, \
             index {} ; synthèse {} / {} / {} ; perte {} ; sauts d'index {}\n\n",
            config.read_length,
            index_read_length,
            config.coverage,
            percent(config.sequencing.substitution),
            percent(config.sequencing.insertion),
            percent(config.sequencing.deletion),
            percent(config.index_sequencing.substitution),
            percent(config.synthesis.substitution),
            percent(config.synthesis.insertion),
            percent(config.synthesis.deletion),
            percent(config.dropout),
            percent(config.index_hopping)
        ));
        text.push_str(&format!(
            "{:<7} {:<24} {:>8} {:>9}  {}\n",
            "Tirage", "Échantillon", "Paires", "Fausses", "Résultat"
        ));
        for trial in &report.tirages {
            let mut first = true;
            for sample in &trial.echantillons {
                let verdict = match sample.recuperation {
                    Recovered::Exact => "récupéré",
                    Recovered::Echec => "échec annoncé",
                };
                text.push_str(&format!(
                    "{} {:<24} {:>8} {:>9}  {}\n",
                    if first {
                        format!("{:<7}", trial.tirage)
                    } else {
                        format!("{:<7}", "")
                    },
                    sample.nom,
                    sample.paires,
                    sample.fausses,
                    verdict
                ));
                first = false;
            }
            if trial.ambigues + trial.inconnues + trial.sautees > 0 {
                text.push_str(&format!(
                    "        sans réponse : {} ambiguës, {} inconnues ; {} sauts d'index\n",
                    trial.ambigues, trial.inconnues, trial.sautees
                ));
            }
        }
        if let Some(dir) = &sequencer.reads {
            text.push_str(&format!(
                "\nLectures du premier tirage écrites dans {} (R1, R2, I1, I2).\n",
                dir.display()
            ));
        }
        let mut fields = Fields::new();
        fields
            .add(
                "Récupérations",
                format!("{recovered} sur {trials} tirages, tous échantillons restitués"),
            )
            .add("Fichiers faux", "0 : un échec est toujours annoncé")
            .add("Durée totale", seconds(elapsed));
        text.push_str(&format!("\n{}\n", fields.render(0)));
        text
    })
}
/// Un fichier FASTQ de lectures simulées : les qualités y sont inscrites en Phred+33.
struct ReadWriter(FastqWriter);

impl ReadWriter {
    fn write(&mut self, header: &str, bases: &[u8], phred: &[u8]) -> Result<()> {
        let quality: Vec<u8> = phred.iter().map(|&score| score + 33).collect();
        Ok(self.0.write(header, bases, &quality)?)
    }

    fn finish(self) -> Result<()> {
        self.0.finish()?;
        Ok(())
    }
}

/// Écrit les lectures d'un tirage en FASTQ : `R1.fastq`, `R2.fastq`, `I1.fastq` et `I2.fastq`.
/// Chaque en-tête porte l'échantillon d'origine, pour `adn demux --truth`.
fn write_reads(
    dir: &Path,
    library: &Library,
    molecules: &[PoolMolecule],
    pairs: &[ReadPair],
    ids: &[String],
) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("impossible de créer {}", dir.display()))?;
    let open = |name: &str| -> Result<ReadWriter> {
        FastqWriter::create(dir.join(name))
            .map(ReadWriter)
            .with_context(|| format!("impossible d'écrire {}", dir.join(name).display()))
    };
    let mut r1 = open("R1.fastq")?;
    let mut r2 = open("R2.fastq")?;
    let mut i1 = open("I1.fastq")?;
    let mut i2 = if pairs.first().is_some_and(|pair| pair.i5.is_some()) {
        Some(open("I2.fastq")?)
    } else {
        None
    };
    for (rank, pair) in pairs.iter().enumerate() {
        let id = &ids[rank];
        let sample = library
            .sample(molecules[pair.molecule as usize].sample)
            .name
            .as_str();
        r1.write(
            &read_header(id, 1, pair, Some(sample)),
            &pair.r1,
            &pair.quality_r1,
        )?;
        r2.write(
            &read_header(id, 2, pair, Some(sample)),
            &pair.r2,
            &pair.quality_r2,
        )?;
        i1.write(
            &read_header(id, 3, pair, Some(sample)),
            &pair.i7,
            &pair.quality_i7,
        )?;
        if let (Some(writer), Some(bases), Some(quality)) = (&mut i2, &pair.i5, &pair.quality_i5) {
            writer.write(&read_header(id, 4, pair, Some(sample)), bases, quality)?;
        }
    }
    r1.finish()?;
    r2.finish()?;
    i1.finish()?;
    if let Some(writer) = i2 {
        writer.finish()?;
    }
    Ok(())
}
