//! `adn encode` : archive un fichier en séquences d'ADN.

use crate::output::{bytes, seconds, thousands, Fields, Printer};
use adn_core::{encode, fasta, Scheme};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(crate) fn run(
    input: &Path,
    output: Option<PathBuf>,
    codec: &crate::CodecArgs,
    out: &Printer,
) -> Result<()> {
    let options = codec.options()?;
    let data =
        std::fs::read(input).with_context(|| format!("impossible de lire {}", input.display()))?;
    let archive = archive_path(output, input);
    if archive == input {
        bail!("l'archive écraserait le fichier d'entrée : indiquer un autre chemin avec -o");
    }

    let started = Instant::now();
    let encoded = encode(&data, &options).context("encodage impossible")?;
    let elapsed = started.elapsed().as_secs_f64();
    let stats = &encoded.stats;

    if let Some(parent) = archive.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("impossible de créer le répertoire {}", parent.display())
            })?;
        }
    }
    let oligo_length = options.oligo_length();
    let written = fasta::write_archive(
        &archive,
        &encoded.oligos,
        options.scheme,
        stats.manifest_copies,
    )
    .with_context(|| format!("impossible d'écrire {}", archive.display()))?;

    let manifest_length = encoded.oligos[0].len();
    let droplets_per_unit = stats.data_oligos as f64 / f64::from(stats.units);
    let redundancy = match options.scheme {
        Scheme::Fountain => format!(
            "marge {:.2} ; {:.2} gouttes par bloc",
            options.fountain.redundancy, droplets_per_unit
        ),
        Scheme::Strands => format!(
            "{} brins de parité par bloc de {}",
            adn_core::codec::strands::OUTER_PARITY,
            adn_core::codec::strands::OUTER_DATA
        ),
        Scheme::Goldman => "recouvrement ×4 : fenêtres de 100 nt décalées de 25".to_string(),
        Scheme::Grass => format!(
            "parité interne de {} symboles ; {} brins de parité par bloc de {}",
            options.grass.parity,
            adn_core::codec::grass::BLOCK_PARITY,
            adn_core::codec::grass::BLOCK
        ),
        Scheme::Ldpc => format!(
            "parité LDPC de {} % : {} brins de parité par bloc de {}",
            options.ldpc.parity,
            adn_core::codec::ldpc::parity_of(
                adn_core::codec::ldpc::BLOCK_DATA,
                options.ldpc.parity,
            ),
            adn_core::codec::ldpc::BLOCK_DATA
        ),
    };
    let sha256 = adn_core::checksum::hex(&stats.sha256);
    let lengths = if manifest_length == oligo_length {
        format!("de {oligo_length} nt chacun")
    } else {
        format!("de {oligo_length} nt, manifestes de {manifest_length} nt")
    };

    let report = serde_json::json!({
        "input": input,
        "output": written,
        "scheme": options.scheme,
        "oligo_length_nt": oligo_length,
        "compression": stats.compression,
        "original_bytes": stats.original_len,
        "body_bytes": stats.body_len,
        "units": stats.units,
        "data_oligos": stats.data_oligos,
        "manifest_copies": stats.manifest_copies,
        "nucleotides": stats.nucleotides,
        "bits_per_nucleotide": stats.bits_per_nucleotide,
        "droplets_per_unit": droplets_per_unit,
        "candidates_tried": stats.candidates_tried,
        "encode_seconds": elapsed,
        "sha256": sha256,
    });
    let input_name = crate::commands::name(input);
    out.finish(&report, || {
        let mut fields = Fields::new();
        fields
            .add(
                "Fichier",
                format!("{} ({})", input_name, bytes(data.len() as u64)),
            )
            .add("Schéma", options.scheme.label())
            .add(
                "Compression",
                format!(
                    "{} (corps de {})",
                    stats.compression.name(),
                    bytes(stats.body_len)
                ),
            )
            .add(
                "Oligos",
                format!(
                    "{} au total : {} de données et {} manifestes, {lengths}",
                    thousands(encoded.oligos.len() as u64),
                    thousands(stats.data_oligos as u64),
                    thousands(stats.manifest_copies as u64),
                ),
            )
            .add("Nucléotides", thousands(stats.nucleotides))
            .add(
                "Densité",
                format!("{:.3} bits par nucléotide", stats.bits_per_nucleotide),
            )
            .add("Redondance", redundancy)
            .add("SHA-256", &sha256)
            .add("Durée", seconds(elapsed));
        format!(
            "Encodage de {} :\n{}\nArchive écrite : {}\n",
            input_name,
            fields.render(2),
            written.display()
        )
    })
}

/// Chemin de l'archive : celui demandé s'il nomme un fichier FASTA, sinon un fichier nommé
/// d'après l'entrée et placé dans le répertoire demandé (ou à côté de l'entrée).
fn archive_path(output: Option<PathBuf>, input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("archive");
    let path = match output {
        Some(path) => {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let is_archive = [".fasta", ".fa", ".fas"]
                .iter()
                .any(|suffix| name.to_ascii_lowercase().ends_with(suffix));
            if path.is_dir() || !is_archive {
                path.join(format!("{stem}.fasta"))
            } else {
                path
            }
        }
        None => input.with_extension("fasta"),
    };
    if path == input {
        input.with_extension("adn.fasta")
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_follows_the_request() {
        let input = Path::new("data/lettre.txt");
        assert_eq!(
            archive_path(None, input),
            PathBuf::from("data/lettre.fasta")
        );
        assert_eq!(
            archive_path(Some("sortie.fasta".into()), input),
            PathBuf::from("sortie.fasta")
        );
        assert_eq!(
            archive_path(Some("sortie".into()), input),
            PathBuf::from("sortie/lettre.fasta")
        );
        assert_eq!(
            archive_path(Some("x.FA".into()), Path::new("d")),
            PathBuf::from("x.FA")
        );
        // Encoder une archive ne doit pas la remplacer silencieusement.
        let input = Path::new("arc.fasta");
        assert_eq!(archive_path(None, input), PathBuf::from("arc.adn.fasta"));
    }
}
