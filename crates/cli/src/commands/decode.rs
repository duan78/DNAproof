//! `adn decode` : restitue le fichier depuis des lectures.

use crate::output::{bytes, Fields, Printer};
use adn_core::{checksum, decode, fasta, DecodeOptions};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub(crate) fn run(
    inputs: &[PathBuf],
    output: Option<PathBuf>,
    algorithm: Option<crate::SchemeArg>,
    expect_sha256: Option<&str>,
    out: &Printer,
) -> Result<()> {
    let records = crate::commands::read_reads(inputs)?;
    let expected = match expect_sha256 {
        Some(text) => Some(
            checksum::parse_sha256(text)
                .with_context(|| format!("empreinte attendue invalide : {text}"))?,
        ),
        None => None,
    };
    let options = DecodeOptions {
        scheme: algorithm.map(crate::SchemeArg::scheme),
    };

    let decoded = decode(crate::commands::sequences(&records), &options)
        .context("le fichier n'a pas pu être restitué ; rien n'a été écrit")?;
    // Le fichier rendu porte déjà l'empreinte embarquée dans l'archive, vérifiée. L'empreinte
    // attendue, quand elle est donnée, est un contrôle indépendant.
    if let Some(expected) = expected {
        if decoded.sha256 != expected {
            bail!(
                "l'empreinte du fichier restitué ({}) ne correspond pas à celle attendue ({}) ; rien n'a été écrit",
                checksum::hex(&decoded.sha256),
                checksum::hex(&expected)
            );
        }
    }

    let output = output.unwrap_or_else(|| default_output(&inputs[0]));
    if inputs.contains(&output) {
        bail!(
            "le fichier à écraser ({}) est une des entrées ; indiquer un autre chemin avec -o",
            output.display()
        );
    }
    fasta::write_atomic(&output, &decoded.data)
        .with_context(|| format!("impossible d'écrire {}", output.display()))?;

    let stats = &decoded.stats;
    let sha256 = checksum::hex(&decoded.sha256);
    let report = serde_json::json!({
        "inputs": inputs,
        "output": output,
        "scheme": decoded.scheme,
        "compression": decoded.compression,
        "bytes": decoded.data.len(),
        "sha256": sha256,
        "expected_sha256_matches": expected.is_none_or(|expected| expected == decoded.sha256),
        "reads": {
            "total": stats.reads_total,
            "invalid": stats.reads_invalid,
            "wrong_length": stats.reads_bad_length,
            "manifest_candidates": stats.manifest_candidates,
            "manifest_agreeing": stats.manifest_agreeing,
            "corrected": stats.corrected,
            "uncorrectable": stats.uncorrectable,
        },
        "decoding": {
            "units_used": stats.units_used,
            "contested": stats.contested,
            "blocks_repaired": stats.blocks_repaired,
            "strands_repaired": stats.strands_repaired,
            "retried_without_corrected_reads": stats.retried_without_corrected_reads,
        },
    });
    let names: Vec<String> = inputs
        .iter()
        .map(|path| crate::commands::name(path))
        .collect();
    out.finish(&report, || {
        let mut fields = Fields::new();
        fields
            .add("Schéma", decoded.scheme.label())
            .add("Compression", decoded.compression.name())
            .add(
                "Lectures",
                format!(
                    "{} ({}) ; {} manifestes confirmants",
                    stats.reads_total - stats.reads_invalid,
                    stats.reads_total,
                    stats.manifest_agreeing
                ),
            )
            .add("Corrigées", format!("{} lectures", stats.corrected))
            .add(
                "Irrécupérables",
                format!("{} lectures", stats.uncorrectable),
            );
        if stats.contested > 0 {
            fields.add(
                "Écartées",
                format!("{} oligos aux lectures contradictoires", stats.contested),
            );
        }
        if stats.reads_invalid + stats.reads_bad_length > 0 {
            fields.add(
                "Ignorées",
                format!(
                    "{} illisibles, {} de mauvaise longueur",
                    stats.reads_invalid, stats.reads_bad_length
                ),
            );
        }
        if stats.strands_repaired > 0 {
            fields.add(
                "Réparées",
                format!(
                    "{} brins restitués par le code externe",
                    stats.strands_repaired
                ),
            );
        }
        if stats.retried_without_corrected_reads {
            fields.add(
                "Second passage",
                "sans les lectures corrigées par la parité",
            );
        }
        fields
            .add(
                "Fichier",
                format!(
                    "{} ({})",
                    output.display(),
                    bytes(decoded.data.len() as u64)
                ),
            )
            .add("SHA-256", &sha256);
        let check = if expected.is_some() {
            "L'empreinte attendue correspond.\n"
        } else {
            "L'empreinte embarquée dans l'archive correspond.\n"
        };
        format!(
            "Décodage de {} :\n{}\n{}",
            names.join(", "),
            fields.render(2),
            check
        )
    })
}

/// Fichier à écrire par défaut : `<première entrée sans .fasta>.decoded`.
fn default_output(first: &Path) -> PathBuf {
    let name = crate::commands::name(first);
    let lower = name.to_ascii_lowercase();
    let stem = [".fasta", ".fa", ".fas"]
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix).map(|stem| stem.len()))
        .unwrap_or(name.len());
    let mut path = first.to_path_buf();
    path.set_file_name(format!("{}.decoded", &name[..stem]));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_strips_the_fasta_suffix() {
        assert_eq!(
            default_output(Path::new("a/arc.fasta")),
            PathBuf::from("a/arc.decoded")
        );
        assert_eq!(
            default_output(Path::new("a/ARC.FA")),
            PathBuf::from("a/ARC.decoded")
        );
        assert_eq!(
            default_output(Path::new("lectures.txt")),
            PathBuf::from("lectures.txt.decoded")
        );
    }
}
