//! `adn demux` : sépare les lectures d'un séquenceur par échantillon, puis coupe les adaptateurs.
//!
//! Une paire de lectures est rangée d'après ses lectures d'index : à une erreur près par index,
//! dans les deux sens de lecture. Une paire équidistante de deux échantillons reste sans réponse,
//! jamais à moitié rangée. Les lectures rangées sont coupées des adaptateurs, la lecture 2
//! remise dans le sens de l'insert : chaque échantillon se décode comme une archive
//! (`adn decode <échantillon>.fasta`).
//!
//! Les lectures simulées portent l'échantillon d'origine dans leur en-tête : `--truth` le
//! compare au rangement, et compte les fausses attributions.

use crate::output::{thousands, Printer};
use adn_core::fasta::Writer;
use adn_core::library::{place, Library, Placement};
use adn_simulation::truth_sample;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

/// Un échantillon connu par ses index : `<i7>[+<i5>]`, ou `<i7>[+<i5>]=<nom>`. Chaque index est
/// un nom Nextera ou une séquence.
#[derive(Clone, Debug)]
pub struct DemuxSampleArg {
    i7: String,
    i5: Option<String>,
    name: String,
}

impl FromStr for DemuxSampleArg {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        const EXPECTED: &str = "attendu : <i7>[+<i5>], ou <i7>[+<i5>]=<nom>";
        let parts: Vec<&str> = text.split('=').collect();
        let (indexes, name) = match parts.as_slice() {
            [indexes] => (*indexes, *indexes),
            [indexes, name] => (*indexes, *name),
            _ => return Err(format!("{text} : {EXPECTED}")),
        };
        let (i7, i5) = match indexes.split_once('+') {
            Some((i7, i5)) => (i7, Some(i5)),
            None => (indexes, None),
        };
        if i7.is_empty() || name.is_empty() || i5 == Some("") {
            return Err(format!("{text} : {EXPECTED}"));
        };
        Ok(Self {
            i7: i7.to_string(),
            i5: i5.map(str::to_string),
            name: name.to_string(),
        })
    }
}

/// Une paire de lectures, telles que lues, prête à être rangée.
#[derive(Clone, Copy, Debug)]
pub struct RawPair<'a> {
    /// Identifiant commun aux lectures de la paire.
    pub id: &'a str,
    pub r1: &'a [u8],
    pub r2: &'a [u8],
    pub i7: &'a [u8],
    pub i5: Option<&'a [u8]>,
    /// L'échantillon d'origine, quand il est connu.
    pub origin: Option<&'a str>,
}

/// Le rangement des paires d'un échantillon.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SampleCounts {
    pub nom: String,
    pub fichier: Option<String>,
    /// Paires rangées dans l'échantillon.
    pub paires: usize,
    /// Lectures écrites : les deux d'une paire quand elles contiennent de l'insert.
    pub lectures: usize,
    /// Erreurs comptées dans les index des paires rangées.
    pub erreurs_index: usize,
}

/// Le bilan d'un démultiplexage.
#[derive(Debug, Default, Serialize)]
pub struct Demultiplexed {
    pub paires: usize,
    /// En parallèle des échantillons de la bibliothèque.
    pub echantillons: Vec<SampleCounts>,
    pub ambigues: usize,
    pub inconnues: usize,
    /// Lectures 1 et 2 coupées d'un adaptateur.
    pub coupees_r1: usize,
    pub coupees_r2: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verite: Option<Truth>,
}

/// Le rangement confronté à l'origine des lectures.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Truth {
    pub justes: usize,
    /// Lectures rangées dans un autre échantillon que le leur.
    pub fausses: usize,
    /// Confusions, de l'échantillon d'origine vers celui où la lecture a été rangée.
    pub confusions: Vec<Confusion>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Confusion {
    pub origine: String,
    pub range: String,
    pub lectures: usize,
}

/// Ce qu'une lecture rangée devient : rang de son échantillon, en-tête (`<id>/1` ou `<id>/2`)
/// et séquence, dans le sens de l'insert.
type ReadSink<'a> = dyn FnMut(usize, &str, &[u8]) -> Result<()> + 'a;

/// Range les paires et coupe les adaptateurs. Chaque lecture qui contient de l'insert passe à
/// `sink`.
///
/// Avec `truth_sample` donné pour une paire, le rangement est confronté à l'origine.
pub fn demultiplex(
    library: &Library,
    pairs: &[RawPair<'_>],
    sink: &mut ReadSink<'_>,
) -> Result<Demultiplexed> {
    let mut report = Demultiplexed {
        paires: pairs.len(),
        echantillons: library
            .samples()
            .iter()
            .map(|sample| SampleCounts {
                nom: sample.name.clone(),
                ..SampleCounts::default()
            })
            .collect(),
        ..Demultiplexed::default()
    };
    let with_truth = pairs.iter().any(|pair| pair.origin.is_some());
    if with_truth {
        report.verite = Some(Truth::default());
    }
    let mut confusions: BTreeMap<(String, String), usize> = BTreeMap::new();
    for pair in pairs {
        match place(library, pair.r1, pair.r2, pair.i7, pair.i5) {
            Placement::Sample(reads) => {
                let counts = &mut report.echantillons[reads.sample];
                counts.paires += 1;
                if reads.r1.len() < pair.r1.len() {
                    report.coupees_r1 += 1;
                }
                if pair.r2.len().saturating_sub(19) > reads.r2.len() {
                    report.coupees_r2 += 1;
                }
                if !reads.r1.is_empty() {
                    counts.lectures += 1;
                    sink(reads.sample, &format!("{}/1", pair.id), &reads.r1)?;
                }
                if !reads.r2.is_empty() {
                    counts.lectures += 1;
                    sink(reads.sample, &format!("{}/2", pair.id), &reads.r2)?;
                }
                if let Some(origin) = pair.origin {
                    let truth = report.verite.as_mut().unwrap();
                    let assigned = library.sample(reads.sample).name.clone();
                    if origin == assigned {
                        truth.justes += 1;
                    } else {
                        truth.fausses += 1;
                        *confusions
                            .entry((origin.to_string(), assigned))
                            .or_default() += 1;
                    }
                }
            }
            Placement::Ambiguous => report.ambigues += 1,
            Placement::Unknown => report.inconnues += 1,
        }
    }
    if let Some(truth) = &mut report.verite {
        truth.confusions = confusions
            .into_iter()
            .map(|((origine, range), lectures)| Confusion {
                origine,
                range,
                lectures,
            })
            .collect();
    }
    Ok(report)
}

/// L'identifiant d'une lecture : le premier mot de son en-tête.
fn read_id(header: &str) -> &str {
    header.split_whitespace().next().unwrap_or("")
}

/// La bibliothèque des échantillons déclarés à la commande.
fn declared(samples: &[DemuxSampleArg]) -> Result<Library> {
    let mut built = Vec::with_capacity(samples.len());
    for sample in samples {
        if sample.name.len() > 120
            || matches!(sample.name.as_str(), "." | "..")
            || sample
                .name
                .chars()
                .any(|c| "/\\:*?\"<>|".contains(c) || c.is_control())
        {
            bail!(
                "« {} » ne convient pas comme nom de fichier ; choisir un autre nom : \
                 --sample <i7>+<i5>=<nom>",
                sample.name
            );
        }
        built.push(adn_core::Sample {
            name: sample.name.clone(),
            i7: crate::commands::index(&sample.i7)?,
            i5: sample
                .i5
                .as_deref()
                .map(crate::commands::index)
                .transpose()?,
        });
    }
    Ok(Library::new(built)?)
}

/// Ce que la commande a reçu.
pub(crate) struct DemuxArgs<'a> {
    pub r1_path: &'a Path,
    pub r2_path: &'a Path,
    pub i7_path: &'a Path,
    pub i5_path: Option<&'a Path>,
    pub library_path: Option<&'a Path>,
    pub samples: &'a [DemuxSampleArg],
    pub output: &'a Path,
    pub truth: bool,
}

pub(crate) fn run(args: &DemuxArgs<'_>, out: &Printer) -> Result<()> {
    let DemuxArgs {
        r1_path,
        r2_path,
        i7_path,
        i5_path,
        library_path,
        samples,
        output,
        truth,
    } = *args;
    let library = match (library_path, samples.is_empty()) {
        (Some(_), false) => bail!("donner --library ou des --sample, pas les deux"),
        (Some(path), true) => {
            let records = crate::commands::read_reads(&[path.to_path_buf()])?;
            adn_core::library::read_pool(&records)
                .with_context(|| format!("{} n'est pas une bibliothèque", path.display()))?
                .0
        }
        (None, true) => {
            bail!(
                "aucun échantillon connu : donner --library <pool.fasta>, ou --sample \
                 <i7>[+<i5>][=<nom>]"
            )
        }
        (None, false) => declared(samples)?,
    };
    let read = |path: &Path| crate::commands::read_reads(&[path.to_path_buf()]);
    let r1 = read(r1_path)?;
    let r2 = read(r2_path)?;
    let i7 = read(i7_path)?;
    let i5 = i5_path.map(read).transpose()?;
    let counts = [
        r1.len(),
        r2.len(),
        i7.len(),
        i5.as_ref().map_or(r1.len(), |records| records.len()),
    ];
    if counts.iter().any(|&count| count != counts[0]) {
        bail!(
            "les fichiers de lectures n'ont pas le même nombre de lectures : R1 {}, R2 {}, i7 {}{}",
            counts[0],
            counts[1],
            counts[2],
            i5.as_ref()
                .map(|records| format!(", i5 {}", records.len()))
                .unwrap_or_default()
        );
    }
    let pairs: Vec<RawPair<'_>> = (0..r1.len())
        .map(|rank| {
            let id = read_id(&r1[rank].header);
            let others = [("R2", &r2[rank]), ("i7", &i7[rank])];
            for (label, record) in &others {
                let other = read_id(&record.header);
                if other != id {
                    bail!("les lectures {label} et R1 ne sont pas appariées au rang {} : {other} face à {id}", rank + 1);
                }
            }
            if let Some(records) = &i5 {
                let other = read_id(&records[rank].header);
                if other != id {
                    bail!("les lectures i5 et R1 ne sont pas appariées au rang {} : {other} face à {id}", rank + 1);
                }
            }
            let origin = truth.then(|| truth_sample(&r1[rank].header)).flatten();
            if truth && origin.is_none() {
                bail!(
                    "la lecture {id} ne porte pas l'échantillon d'origine dans son en-tête ; \
                     --truth s'applique aux lectures d'`adn simulate --library`"
                );
            }
            Ok(RawPair {
                id,
                r1: &r1[rank].sequence,
                r2: &r2[rank].sequence,
                i7: &i7[rank].sequence,
                i5: i5.as_ref().map(|records| records[rank].sequence.as_slice()),
                origin,
            })
        })
        .collect::<Result<_>>()?;

    std::fs::create_dir_all(output)
        .with_context(|| format!("impossible de créer {}", output.display()))?;
    let mut writers: Vec<Option<Writer>> = library.samples().iter().map(|_| None).collect();
    let paths: Vec<String> = library
        .samples()
        .iter()
        .map(|sample| {
            output
                .join(format!("{}.fasta", sample.name))
                .display()
                .to_string()
        })
        .collect();
    let mut report = demultiplex(&library, &pairs, &mut |sample, id, sequence| {
        let path = &paths[sample];
        if writers[sample].is_none() {
            writers[sample] =
                Some(Writer::create(path).with_context(|| {
                    format!("impossible d'écrire {}", Path::new(path).display())
                })?);
        }
        writers[sample]
            .as_mut()
            .unwrap()
            .write(id, sequence)
            .map_err(anyhow::Error::from)
    })?;
    for (rank, writer) in writers.into_iter().enumerate() {
        if let Some(writer) = writer {
            writer.finish()?;
            report.echantillons[rank].fichier = Some(paths[rank].clone());
        }
    }

    out.finish(&report, || {
        let mut text = format!(
            "Démultiplexage de {} : {} échantillons, sortie dans {}\n\n",
            accord(report.paires, "paire de lectures", "paires de lectures"),
            library.samples().len(),
            output.display()
        );
        text.push_str(&format!(
            "{:<24} {:>10} {:>12}  {}\n",
            "Échantillon", "Paires", "Lectures", "Fichier"
        ));
        for sample in &report.echantillons {
            text.push_str(&format!(
                "{:<24} {:>10} {:>12}  {}\n",
                sample.nom,
                thousands(sample.paires as u64),
                thousands(sample.lectures as u64),
                sample.fichier.as_deref().unwrap_or("aucune lecture")
            ));
        }
        text.push_str(&format!(
            "{:<24} {:>10}\n",
            "Sans réponse (ambiguës)",
            thousands(report.ambigues as u64)
        ));
        text.push_str(&format!(
            "{:<24} {:>10}\n",
            "Sans réponse (inconnues)",
            thousands(report.inconnues as u64)
        ));
        text.push_str(&format!(
            "{:<24} {:>10}  lectures 1 ; {} lectures 2\n",
            "Découpées d'un adaptateur",
            thousands(report.coupees_r1 as u64),
            thousands(report.coupees_r2 as u64)
        ));
        if let Some(truth) = &report.verite {
            text.push_str(&format!(
                "{:<24} {:>10}  justes ; {} fausses\n",
                "Origine des lectures",
                thousands(truth.justes as u64),
                thousands(truth.fausses as u64)
            ));
            for confusion in &truth.confusions {
                text.push_str(&format!(
                    "    {} → {} : {} lectures\n",
                    confusion.origine,
                    confusion.range,
                    thousands(confusion.lectures as u64)
                ));
            }
        }
        text.push_str("Chaque échantillon se décode : adn decode <fichier>\n");
        text
    })
}

/// Accorde un nom avec son nombre : « 1 paire de lectures », « 12 paires de lectures ».
fn accord(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{} {singular}", thousands(count as u64))
    } else {
        format!("{} {plural}", thousands(count as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_arguments_are_parsed() {
        let both: DemuxSampleArg = "N701+N501".parse().unwrap();
        assert_eq!(both.i7, "N701");
        assert_eq!(both.i5.as_deref(), Some("N501"));
        assert_eq!(both.name, "N701+N501");
        let named: DemuxSampleArg = "TAAGGCGA+TAGATCGC=photo".parse().unwrap();
        assert_eq!(named.name, "photo");
        assert_eq!(named.i7, "TAAGGCGA");
        let single: DemuxSampleArg = "N703=soleil".parse().unwrap();
        assert_eq!(single.i5, None);
        for bad in ["", "=nom", "N701=", "a=b=c", "+N501"] {
            assert!(bad.parse::<DemuxSampleArg>().is_err(), "{bad}");
        }
        assert!(declared(&["N701=../ailleurs".parse().unwrap()]).is_err());
        assert!(declared(&["N701+N501".parse().unwrap()]).is_ok());
        assert_eq!(accord(1, "paire", "paires"), "1 paire");
        assert_eq!(accord(1000, "paire", "paires"), "1\u{202F}000 paires");
    }
}
