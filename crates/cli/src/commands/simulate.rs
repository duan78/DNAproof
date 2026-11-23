//! `adn simulate` : dégrade une archive comme le feraient la synthèse, la perte et le
//! séquençage, puis tente de la relire.
//!
//! Le bilan compte les fichiers restitués à l'identique. Un échec est toujours annoncé : un
//! fichier qui ne porte pas l'empreinte de l'archive n'est jamais rendu.

use crate::output::{seconds, Fields, Printer};
use adn_core::{decode, encode, DecodeOptions, Scheme};
use adn_simulation::{ChannelConfig, DnaChannel, ErrorRates};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use std::time::Instant;

/// Résultat d'un tirage.
#[derive(Serialize)]
struct Trial {
    tirage: usize,
    oligos: usize,
    lectures: usize,
    oligos_perdus: u32,
    lectures_intactes: usize,
    lectures_hors_longueur: usize,
    recuperation: Recovered,
    /// Le second passage a été nécessaire (sans les lectures corrigées par la parité).
    second_passage: bool,
    /// Brins restitués par le code externe.
    reparations: usize,
}

#[derive(Serialize)]
enum Recovered {
    Exact,
    Echec,
}

#[derive(Serialize)]
struct Report {
    entree: String,
    mode: &'static str,
    schema: Scheme,
    canal: Canal,
    tirages: Vec<Trial>,
    bilan: Bilan,
}

#[derive(Serialize)]
struct Canal {
    substitution_synthese: f64,
    insertion_synthese: f64,
    deletion_synthese: f64,
    substitution_sequencement: f64,
    insertion_sequencement: f64,
    deletion_sequencement: f64,
    perte: f64,
    couverture: f64,
}

#[derive(Serialize)]
struct Bilan {
    tirages: usize,
    recuperes: usize,
    echecs: usize,
    /// Toujours zéro : un fichier qui ne porte pas l'empreinte de l'archive n'est jamais rendu.
    /// Le champ le rend visible.
    faux: usize,
    duree_totale_s: f64,
}

pub(crate) fn run(
    input: &Path,
    archive: bool,
    codec: &crate::CodecArgs,
    channel_args: &crate::ChannelArgs,
    trials: usize,
    seed: u64,
    out: &Printer,
) -> Result<()> {
    if trials == 0 {
        bail!("--trials doit être au moins 1");
    }
    let config = ChannelConfig {
        synthesis: ErrorRates::new(
            channel_args.synthesis_substitution_rate,
            channel_args.synthesis_insertion_rate,
            channel_args.synthesis_deletion_rate,
        ),
        sequencing: ErrorRates::new(
            channel_args.substitution_rate,
            channel_args.insertion_rate,
            channel_args.deletion_rate,
        ),
        dropout: channel_args.dropout,
        coverage: channel_args.coverage,
        seed,
    };
    // Valide les paramètres du canal.
    DnaChannel::new(config).context("paramètres du canal incohérents")?;

    // Soit un fichier, encodé pour l'occasion ; soit une archive, dont on vérifie d'abord
    // qu'elle est lisible. Sans fichier d'origine, les tirages réussis doivent tous rendre le
    // même fichier : on compare leurs empreintes.
    let (oligos, original, scheme, mode) = if archive {
        let records = crate::commands::read_reads(&[input.to_path_buf()])?;
        let oligos = crate::commands::sequences(&records);
        let intact = decode(&oligos, &DecodeOptions::default())
            .context("l'archive fournie n'est pas lisible en l'état")?;
        (oligos, None, intact.scheme, "archive")
    } else {
        let options = codec.options()?;
        let data = std::fs::read(input)
            .with_context(|| format!("impossible de lire {}", input.display()))?;
        let encoded = encode(&data, &options).context("encodage impossible")?;
        (encoded.oligos, Some(data), options.scheme, "fichier")
    };

    let started = Instant::now();
    let mut results = Vec::with_capacity(trials);
    let mut fingerprints: Vec<[u8; 32]> = Vec::new();
    for trial in 0..trials {
        let config = ChannelConfig {
            seed: seed.wrapping_add(trial as u64),
            ..config
        };
        let (reads, stats) = DnaChannel::new(config)?.simulate(&oligos);
        let sequences: Vec<Vec<u8>> = reads.iter().map(|read| read.sequence.clone()).collect();
        let outcome = decode(&sequences, &DecodeOptions::default());
        let result = match outcome {
            Ok(decoded) => {
                // Le fichier rendu porte l'empreinte de l'archive. Le comparer à l'original
                // contrôle l'ensemble de la chaîne.
                if original.as_ref().is_some_and(|data| *data != decoded.data) {
                    bail!(
                        "tirage {} : le fichier restitué diffère de l'original",
                        trial + 1
                    );
                }
                if fingerprints.iter().any(|seen| *seen != decoded.sha256) {
                    bail!(
                        "tirage {} : deux tirages réussis ont rendu des fichiers différents",
                        trial + 1
                    );
                }
                fingerprints.push(decoded.sha256);
                Trial {
                    tirage: trial + 1,
                    oligos: stats.oligos as usize,
                    lectures: stats.reads,
                    oligos_perdus: stats.lost_oligos,
                    lectures_intactes: stats.reads_perfect,
                    lectures_hors_longueur: stats.reads_wrong_length,
                    recuperation: Recovered::Exact,
                    second_passage: decoded.stats.retried_without_corrected_reads,
                    reparations: decoded.stats.strands_repaired,
                }
            }
            Err(_) => Trial {
                tirage: trial + 1,
                oligos: stats.oligos as usize,
                lectures: stats.reads,
                oligos_perdus: stats.lost_oligos,
                lectures_intactes: stats.reads_perfect,
                lectures_hors_longueur: stats.reads_wrong_length,
                recuperation: Recovered::Echec,
                second_passage: false,
                reparations: 0,
            },
        };
        results.push(result);
    }
    let elapsed = started.elapsed().as_secs_f64();

    let recovered = results
        .iter()
        .filter(|result| matches!(result.recuperation, Recovered::Exact))
        .count();
    let report = Report {
        entree: input.to_string_lossy().to_string(),
        mode,
        schema: scheme,
        canal: Canal {
            substitution_synthese: config.synthesis.substitution,
            insertion_synthese: config.synthesis.insertion,
            deletion_synthese: config.synthesis.deletion,
            substitution_sequencement: config.sequencing.substitution,
            insertion_sequencement: config.sequencing.insertion,
            deletion_sequencement: config.sequencing.deletion,
            perte: config.dropout,
            couverture: config.coverage,
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
            "Simulation sur {} ({}) : {} tirages, schéma {}\n",
            crate::commands::name(input),
            if mode == "archive" {
                "archive existante"
            } else {
                "fichier encodé pour l'occasion"
            },
            trials,
            scheme.label()
        );
        text.push_str(&format!(
            "Canal, par nucléotide : synthèse {} / {} / {}, séquençage {} / {} / {} \
             (substitution / insertion / délétion) ; perte {} ; couverture {}\n\n",
            percent(config.synthesis.substitution),
            percent(config.synthesis.insertion),
            percent(config.synthesis.deletion),
            percent(config.sequencing.substitution),
            percent(config.sequencing.insertion),
            percent(config.sequencing.deletion),
            percent(config.dropout),
            config.coverage
        ));
        text.push_str(&format!(
            "{:<7} {:>9} {:>13} {:>10} {:>12}  {}\n",
            "Tirage", "Lectures", "Oligos perdus", "Intactes", "Hors long.", "Résultat"
        ));
        for result in &report.tirages {
            let lost = if result.oligos == 0 {
                String::from("—")
            } else {
                format!(
                    "{:.1} %",
                    result.oligos_perdus as f64 / result.oligos as f64 * 100.0
                )
            };
            let verdict = match result.recuperation {
                Recovered::Exact => "récupéré",
                Recovered::Echec => "échec annoncé",
            };
            let note = match (result.second_passage, result.reparations) {
                (false, 0) => String::new(),
                (true, _) => String::from(" (second passage)"),
                (false, repairs) => format!(" ({repairs} brins réparés)"),
            };
            text.push_str(&format!(
                "{:<7} {:>9} {:>13} {:>10} {:>12}  {}{}\n",
                result.tirage,
                result.lectures,
                lost,
                result.lectures_intactes,
                result.lectures_hors_longueur,
                verdict,
                note
            ));
        }
        let mut fields = Fields::new();
        fields
            .add("Récupérations", format!("{recovered} sur {trials} tirages"))
            .add("Fichiers faux", "0 : un échec est toujours annoncé")
            .add("Durée totale", seconds(elapsed));
        text.push_str(&format!("\n{}\n", fields.render(0)));
        text
    })
}
