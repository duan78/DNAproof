//! `adn library` : range des archives dans les molécules d'une bibliothèque de séquençage.
//!
//! Chaque archive devient un échantillon : ses oligos sont les inserts de molécules au format
//! Nextera, qui portent les index de l'échantillon. Les molécules de tous les échantillons
//! forment un pool, prêt à être séquencé (`adn simulate --library`) et à guider le
//! démultiplexage (`adn demux --library`). Les index sont vérifiés : une erreur par index ne peut
//! pas confondre deux échantillons.

use crate::output::Printer;
use adn_core::fasta::Writer;
use adn_core::library::{Library, Sample};
use adn_core::sequence::BASES;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Un échantillon à assembler : `<archive>=<i7>[+<i5>]`, ou `<nom>=<archive>=<i7>[+<i5>]`.
#[derive(Clone, Debug)]
pub struct SampleArg {
    name: Option<String>,
    archive: PathBuf,
    i7: String,
    i5: Option<String>,
}

impl FromStr for SampleArg {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        const EXPECTED: &str = "attendu : <archive>=<i7>[+<i5>], ou <nom>=<archive>=<i7>[+<i5>]";
        let parts: Vec<&str> = text.split('=').collect();
        let (name, archive, indexes) = match parts.as_slice() {
            [archive, indexes] => (None, *archive, *indexes),
            [name, archive, indexes] => (Some(*name), *archive, *indexes),
            _ => return Err(format!("{text} : {EXPECTED}")),
        };
        if archive.is_empty() || indexes.is_empty() || name.is_some_and(str::is_empty) {
            return Err(format!("{text} : {EXPECTED}"));
        }
        let (i7, i5) = match indexes.split_once('+') {
            Some((i7, i5)) => (i7, Some(i5)),
            None => (indexes, None),
        };
        Ok(Self {
            name: name.map(str::to_string),
            archive: PathBuf::from(archive),
            i7: i7.to_string(),
            i5: i5.map(str::to_string),
        })
    }
}

#[derive(Serialize)]
struct Report {
    sortie: String,
    echantillons: Vec<SampleReport>,
    molecules: usize,
}

#[derive(Serialize)]
struct SampleReport {
    nom: String,
    archive: String,
    i7: String,
    i5: Option<String>,
    oligos: usize,
    insert_min: usize,
    insert_max: usize,
    molecule_min: usize,
    molecule_max: usize,
}

/// Le nom d'un fichier sans sa dernière extension.
fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| crate::commands::name(path))
}

pub(crate) fn run(samples: &[SampleArg], output: &Path, out: &Printer) -> Result<()> {
    if samples.is_empty() {
        bail!("aucun échantillon : donner --sample <archive>=<i7>[+<i5>] au moins une fois");
    }
    if samples.iter().any(|sample| sample.archive == output) {
        bail!(
            "le fichier à écrire ({}) est une des archives ; choisir un autre chemin avec --output",
            output.display()
        );
    }
    let mut loaded = Vec::with_capacity(samples.len());
    for sample in samples {
        let records = crate::commands::read_reads(std::slice::from_ref(&sample.archive))?;
        let mut oligos = Vec::with_capacity(records.len());
        for (rank, record) in records.iter().enumerate() {
            if let Some(&base) = record.sequence.iter().find(|&&base| !BASES.contains(&base)) {
                bail!(
                    "{} : l'oligo {rank} contient un caractère qui n'est pas A, C, G, T ({}) ; \
                     ce n'est pas une archive",
                    crate::commands::name(&sample.archive),
                    char::from(base)
                );
            }
            oligos.push(record.sequence.clone());
        }
        let name = sample.name.clone().unwrap_or_else(|| stem(&sample.archive));
        loaded.push((name, sample, oligos));
    }
    let library = Library::new(
        loaded
            .iter()
            .map(|(name, sample, _)| -> Result<Sample> {
                Ok(Sample {
                    name: name.clone(),
                    i7: crate::commands::index(&sample.i7)?,
                    i5: sample
                        .i5
                        .as_deref()
                        .map(crate::commands::index)
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    )?;

    let mut writer = Writer::create(output)
        .with_context(|| format!("impossible d'écrire {}", output.display()))?;
    let mut reports = Vec::with_capacity(loaded.len());
    let mut molecules = 0;
    for sample in library.samples() {
        let rank = library.rank(&sample.name).unwrap_or(0);
        let (_, arg, oligos) = loaded
            .iter()
            .find(|(name, _, _)| *name == sample.name)
            .ok_or_else(|| anyhow::anyhow!("échantillon sans archive : {}", sample.name))?;
        let header = library.molecule_header(rank)?;
        let (insert_min, insert_max) = oligos.iter().fold((usize::MAX, 0), |(low, high), oligo| {
            (low.min(oligo.len()), high.max(oligo.len()))
        });
        let (before, after) = library.primer_offsets(rank)?;
        for oligo in oligos {
            writer.write(&header, &library.molecule(rank, oligo)?)?;
        }
        molecules += oligos.len();
        reports.push(SampleReport {
            nom: sample.name.clone(),
            archive: arg.archive.display().to_string(),
            i7: sample.i7.describe(),
            i5: sample.i5.as_ref().map(|index| index.describe()),
            oligos: oligos.len(),
            insert_min,
            insert_max,
            molecule_min: before + insert_min + after,
            molecule_max: before + insert_max + after,
        });
    }
    writer.finish()?;

    let report = Report {
        sortie: output.display().to_string(),
        echantillons: reports,
        molecules,
    };
    out.finish(&report, || {
        let mut text = format!(
            "Bibliothèque de {} écrit dans {} : {} molécules\n\n",
            if samples.len() == 1 {
                "un échantillon".to_string()
            } else {
                format!("{} échantillons", samples.len())
            },
            output.display(),
            crate::output::thousands(molecules as u64)
        );
        text.push_str(&format!(
            "{:<24} {:<33} {:>8} {:<15} {}\n",
            "Échantillon", "Index", "Oligos", "Insert", "Molécule"
        ));
        for sample in &report.echantillons {
            let indexes = match &sample.i5 {
                Some(i5) => format!(
                    "i7 {}  i5 {}",
                    sample.i7.replace('=', " "),
                    i5.replace('=', " ")
                ),
                None => format!("i7 {}", sample.i7.replace('=', " ")),
            };
            let insert = if sample.insert_min == sample.insert_max {
                format!("{} nt", sample.insert_min)
            } else {
                format!("{} à {} nt", sample.insert_min, sample.insert_max)
            };
            let molecule = if sample.molecule_min == sample.molecule_max {
                format!("{} nt", sample.molecule_min)
            } else {
                format!("{} à {} nt", sample.molecule_min, sample.molecule_max)
            };
            text.push_str(&format!(
                "{:<24} {:<33} {:>8} {:<15} {}\n",
                sample.nom,
                indexes,
                crate::output::thousands(sample.oligos as u64),
                insert,
                molecule
            ));
        }
        text.push_str(&format!(
            "\nPool prêt pour `adn simulate --library {}` et `adn demux --library {}`.\n",
            output.display(),
            output.display()
        ));
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_arguments_are_parsed() {
        let plain: SampleArg = "a.fasta=N701+N501".parse().unwrap();
        assert_eq!(plain.archive, PathBuf::from("a.fasta"));
        assert_eq!(plain.name, None);
        assert_eq!(plain.i7, "N701");
        assert_eq!(plain.i5.as_deref(), Some("N501"));
        let named: SampleArg = "doc=C:/archives/doc.adn.fasta=TAAGGCGA".parse().unwrap();
        assert_eq!(named.name.as_deref(), Some("doc"));
        assert_eq!(named.archive, PathBuf::from("C:/archives/doc.adn.fasta"));
        assert_eq!(named.i5, None);
        for bad in ["a.fasta", "=a.fasta=N701", "a.fasta=", "doc==N701", "N701"] {
            assert!(bad.parse::<SampleArg>().is_err(), "{bad}");
        }
        assert_eq!(stem(Path::new("d/doc.pdf.adn.fasta")), "doc.pdf.adn");
        assert_eq!(stem(Path::new("archive")), "archive");
    }
}
