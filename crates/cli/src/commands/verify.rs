//! `adn verify` : vérifie une archive sans rien écrire, sauf fichier demandé.

use crate::output::{thousands, Fields, Printer};
use adn_core::{checksum, decode, fasta, DecodeOptions, DnaError};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(crate) fn run(
    input: &Path,
    limits: &crate::ConstraintArgs,
    output: Option<PathBuf>,
    out: &Printer,
) -> Result<()> {
    let constraints = limits.constraints()?;
    let records = crate::commands::read_reads(&[input.to_path_buf()])?;
    let pool = adn_core::verify::pool_report(&records, &constraints);
    let decoded = decode(
        crate::commands::sequences(&records),
        &DecodeOptions::default(),
    );

    let (verdict, sha256, written) = match (&decoded, output) {
        (Ok(decoded), Some(path)) => {
            fasta::write_atomic(&path, &decoded.data)
                .with_context(|| format!("impossible d'écrire {}", path.display()))?;
            (
                "décodable ; empreinte vérifiée",
                checksum::hex(&decoded.sha256),
                Some(path.display().to_string()),
            )
        }
        (Ok(decoded), None) => (
            "décodable ; empreinte vérifiée",
            checksum::hex(&decoded.sha256),
            None,
        ),
        (Err(error), _) => (
            match error {
                DnaError::UnsupportedFormat(_) => "pas une archive lisible",
                _ => "illisible en l'état",
            },
            String::new(),
            None,
        ),
    };

    let report = serde_json::json!({
        "input": input,
        "sequences": pool.oligos,
        "nucleotides": pool.nucleotides,
        "length_min": pool.length_min,
        "length_max": pool.length_max,
        "gc_min": pool.gc_min,
        "gc_mean": pool.gc_mean,
        "gc_max": pool.gc_max,
        "max_homopolymer": pool.max_homopolymer,
        "compliant": pool.compliant,
        "constraints": {
            "gc_min": constraints.gc_min,
            "gc_max": constraints.gc_max,
            "max_homopolymer": constraints.max_homopolymer,
        },
        "decodable": decoded.is_ok(),
        "scheme": decoded.as_ref().ok().map(|decoded| decoded.scheme),
        "bytes": decoded.as_ref().ok().map(|decoded| decoded.data.len()),
        "sha256": sha256,
        "error": decoded.as_ref().err().map(ToString::to_string),
        "written": written,
    });
    let name = crate::commands::name(input);
    out.finish(&report, || {
        let mut fields = Fields::new();
        fields
            .add(
                "Séquences",
                format!(
                    "{} ({}), de {} à {} nt",
                    thousands(pool.oligos as u64),
                    name,
                    pool.length_min,
                    pool.length_max
                ),
            )
            .add(
                "GC",
                format!(
                    "de {:.1} à {:.1} %, {:.1} % en moyenne",
                    pool.gc_min * 100.0,
                    pool.gc_max * 100.0,
                    pool.gc_mean * 100.0
                ),
            )
            .add(
                "Contraintes",
                format!(
                    "{}/{} séquences respectent GC {:.0}–{:.0} % et suites ≤ {} (la plus longue : {})",
                    thousands(pool.compliant as u64),
                    thousands(pool.oligos as u64),
                    constraints.gc_min * 100.0,
                    constraints.gc_max * 100.0,
                    constraints.max_homopolymer,
                    pool.max_homopolymer
                ),
            )
            .add("Archive", verdict);
        if let Ok(decoded) = &decoded {
            fields
                .add("Schéma", decoded.scheme.label())
                .add("Fichier", format!("{} octets", thousands(decoded.data.len() as u64)))
                .add("SHA-256", &sha256);
        }
        if let Err(error) = &decoded {
            fields.add("Cause", error);
        }
        let mut text = format!("Vérification de {} :\n{}\n", name, fields.render(2));
        match written {
            Some(path) => text.push_str(&format!("Fichier écrit : {path}\n")),
            None => text.push_str("Aucun fichier écrit.\n"),
        }
        text
    })
}
