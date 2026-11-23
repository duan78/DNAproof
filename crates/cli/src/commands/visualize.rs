//! `adn visualize` : statistiques d'un ensemble de séquences, en tableau, JSON ou HTML.

use crate::output::Printer;
use adn_core::verify::OligoReport;
use adn_core::{fasta, verify};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Nombre de séquences montrées en tableau sans `--all`.
const TABLE_LIMIT: usize = 30;

pub(crate) fn run(
    input: &Path,
    format: crate::FormatArg,
    output: Option<PathBuf>,
    limits: &crate::ConstraintArgs,
    all: bool,
    out: &Printer,
) -> Result<()> {
    let constraints = limits.constraints()?;
    let records = crate::commands::read_reads(&[input.to_path_buf()])?;
    let (pool, per_oligo) = verify::inspect(&records, &constraints);

    let text = match format {
        crate::FormatArg::Json => json(input, &pool, &records, &per_oligo)?,
        crate::FormatArg::Html => html(&pool, &records, &per_oligo),
        crate::FormatArg::Table => table(&pool, &per_oligo, all),
    };

    match output {
        Some(path) => {
            fasta::write_atomic(&path, text.as_bytes())
                .with_context(|| format!("impossible d'écrire {}", path.display()))?;
            if !out.is_json() {
                println!("Écrit : {}", path.display());
            }
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn json(
    input: &Path,
    pool: &verify::PoolReport,
    records: &[fasta::Record],
    per_oligo: &[OligoReport],
) -> Result<String> {
    let sequences: Vec<serde_json::Value> = records
        .iter()
        .zip(per_oligo)
        .map(|(record, stats)| {
            serde_json::json!({
                "header": record.header,
                "length": stats.length,
                "gc": stats.gc,
                "max_homopolymer": stats.max_homopolymer,
                "compliant": stats.compliant,
            })
        })
        .collect();
    let document = serde_json::json!({
        "input": input,
        "pool": pool,
        "sequences": sequences,
    });
    Ok(serde_json::to_string_pretty(&document)?)
}

fn table(pool: &verify::PoolReport, per_oligo: &[OligoReport], all: bool) -> String {
    let shown = if all {
        per_oligo.len()
    } else {
        TABLE_LIMIT.min(per_oligo.len())
    };
    let mut text = format!(
        "Séquences : {}   Nucléotides : {}   Longueurs : {} à {} nt\n",
        pool.oligos, pool.nucleotides, pool.length_min, pool.length_max
    );
    text.push_str(&format!(
        "GC : {:.1} à {:.1} % (moyenne {:.1} %)   Plus longue suite : {}   Conformes : {}/{}\n\n",
        pool.gc_min * 100.0,
        pool.gc_max * 100.0,
        pool.gc_mean * 100.0,
        pool.max_homopolymer,
        pool.compliant,
        pool.oligos
    ));
    text.push_str(&format!(
        "{:<6} {:>8} {:>9} {:>9} {}\n",
        "Rang", "Longueur", "GC", "Suite max", "Conforme"
    ));
    for stats in &per_oligo[..shown] {
        text.push_str(&format!(
            "{:<6} {:>8} {:>8.1} % {:>9} {}\n",
            stats.index,
            stats.length,
            stats.gc * 100.0,
            stats.max_homopolymer,
            if stats.compliant { "oui" } else { "non" }
        ));
    }
    if shown < per_oligo.len() {
        text.push_str(&format!(
            "... {} autres séquences (--all pour toutes)\n",
            per_oligo.len() - shown
        ));
    }
    text
}

/// Échappe le texte destiné au document HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html(pool: &verify::PoolReport, records: &[fasta::Record], per_oligo: &[OligoReport]) -> String {
    let mut rows = String::new();
    for (record, stats) in records.iter().zip(per_oligo) {
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1} %</td><td>{}</td><td>{}</td></tr>\n",
            stats.index,
            escape(&record.header),
            stats.length,
            stats.gc * 100.0,
            stats.max_homopolymer,
            if stats.compliant { "oui" } else { "non" }
        ));
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"fr\"><head><meta charset=\"utf-8\">\n\
<title>Séquences d'ADN</title>\n\
<style>body{{font-family:sans-serif;margin:2rem}}table{{border-collapse:collapse}}\n\
th,td{{border:1px solid #ccc;padding:.3rem .6rem;text-align:left}}th{{background:#eee}}</style>\n\
</head><body>\n<h1>Séquences d'ADN</h1>\n\
<p>{count} séquences, {nucleotides} nucléotides, de {length_min} à {length_max} nt.</p>\n\
<p>GC de {gc_min:.1} à {gc_max:.1} % (moyenne {gc_mean:.1} %) ; plus longue suite de bases \
identiques : {homopolymer} ; {compliant} séquences conformes.</p>\n\
<table><tr><th>Rang</th><th>En-tête</th><th>Longueur</th><th>GC</th><th>Suite max</th>\
<th>Conforme</th></tr>\n{rows}</table>\n</body></html>\n",
        count = pool.oligos,
        nucleotides = pool.nucleotides,
        length_min = pool.length_min,
        length_max = pool.length_max,
        gc_min = pool.gc_min * 100.0,
        gc_max = pool.gc_max * 100.0,
        gc_mean = pool.gc_mean * 100.0,
        homopolymer = pool.max_homopolymer,
        compliant = pool.compliant,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use adn_core::DnaConstraints;

    #[test]
    fn html_is_escaped() {
        let constraints = DnaConstraints::default();
        let records = vec![fasta::Record {
            header: "<script>x</script>".to_string(),
            sequence: b"ACGTTGCAACGTTCGA".to_vec(),
        }];
        let (pool, per_oligo) = verify::inspect(&records, &constraints);
        let page = html(&pool, &records, &per_oligo);
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;x&lt;/script&gt;"));
    }
}
