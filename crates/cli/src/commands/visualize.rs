//! Commande de visualisation

use crate::VisualizationFormat;
use adn_core::{DnaSequence, ConstraintChecker};
use anyhow::Result;
use std::path::PathBuf;
use std::io::{BufRead, BufReader};

pub fn run(input: PathBuf, format: VisualizationFormat, output: Option<PathBuf>) -> Result<()> {
    println!("📊 Visualisation de: {}", input.display());

    // 1. Lire les séquences
    let sequences = read_fasta(&input)?;
    println!("{} séquences chargées", sequences.len());

    // 2. Visualiser selon le format
    match format {
        VisualizationFormat::Table => visualize_table(&sequences)?,
        VisualizationFormat::Json => visualize_json(&sequences, output)?,
        VisualizationFormat::Html => visualize_html(&sequences, output)?,
    }

    Ok(())
}

/// Visualisation en tableau
fn visualize_table(sequences: &[DnaSequence]) -> Result<()> {
    use tabled::{Table, Tabled};

    #[derive(Tabled)]
    struct SequenceRow {
        #[tabled(rename = "ID")]
        id: String,
        #[tabled(rename = "Length")]
        length: usize,
        #[tabled(rename = "GC%")]
        gc_percent: String,
        #[tabled(rename = "Max Homopolymer")]
        max_homopolymer: usize,
        #[tabled(rename = "Entropy")]
        entropy: f64,
    }

    let checker = ConstraintChecker::new();

    let rows: Vec<SequenceRow> = sequences
        .iter()
        .map(|seq| {
            let _stats = checker.stats(&seq.bases);
            SequenceRow {
                id: seq.id.to_string().chars().take(8).collect(),
                length: seq.len(),
                gc_percent: format!("{:.1}%", seq.metadata.gc_ratio * 100.0),
                max_homopolymer: seq.metadata.max_homopolymer,
                entropy: seq.metadata.entropy,
            }
        })
        .collect();

    println!();
    println!("{}", Table::new(rows));

    Ok(())
}

/// Visualisation en JSON
fn visualize_json(sequences: &[DnaSequence], output: Option<PathBuf>) -> Result<()> {
    let checker = ConstraintChecker::new();

    let data: Vec<serde_json::Value> = sequences
        .iter()
        .map(|seq| {
            let stats = checker.stats(&seq.bases);
            serde_json::json!({
                "id": seq.id.to_string(),
                "length": seq.len(),
                "gc_ratio": seq.metadata.gc_ratio,
                "max_homopolymer": seq.metadata.max_homopolymer,
                "entropy": seq.metadata.entropy,
                "chunk_index": seq.metadata.chunk_index,
                "stats": {
                    "count_a": stats.count_a,
                    "count_c": stats.count_c,
                    "count_g": stats.count_g,
                    "count_t": stats.count_t,
                }
            })
        })
        .collect();

    let json = serde_json::to_string_pretty(&data)?;

    if let Some(output) = output {
        std::fs::write(&output, json)?;
        println!("JSON écrit dans: {}", output.display());
    } else {
        println!("\n{}", json);
    }

    Ok(())
}

/// Visualisation en HTML
fn visualize_html(sequences: &[DnaSequence], output: Option<PathBuf>) -> Result<()> {
    let html = format!(
        r#"
<!DOCTYPE html>
<html>
<head>
    <title>ADN Visualization</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        table {{ border-collapse: collapse; width: 100%; }}
        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
        th {{ background-color: #4CAF50; color: white; }}
        tr:nth-child(even) {{ background-color: #f2f2f2; }}
        .stats {{ margin: 20px 0; }}
    </style>
</head>
<body>
    <h1>🧬 ADN Sequence Visualization</h1>
    <div class="stats">
        <p><strong>Total sequences:</strong> {}</p>
    </div>
    <table>
        <tr>
            <th>ID</th>
            <th>Length</th>
            <th>GC%</th>
            <th>Max Homopolymer</th>
            <th>Entropy</th>
        </tr>
        {}
    </table>
</body>
</html>
"#,
        sequences.len(),
        sequences
            .iter()
            .map(|seq| {
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td><td>{:.2}</td></tr>",
                    seq.id.to_string().chars().take(8).collect::<String>(),
                    seq.len(),
                    seq.metadata.gc_ratio * 100.0,
                    seq.metadata.max_homopolymer,
                    seq.metadata.entropy
                )
            })
            .collect::<Vec<_>>()
            .join("\n        ")
    );

    if let Some(output) = output {
        std::fs::write(&output, html)?;
        println!("HTML écrit dans: {}", output.display());
    } else {
        println!("\n{}", html);
    }

    Ok(())
}

/// Lit un fichier FASTA
fn read_fasta(path: &PathBuf) -> Result<Vec<DnaSequence>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut sequences = Vec::new();

    let mut current_seq = String::new();
    let mut chunk_index = 0;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with('>') {
            if !current_seq.is_empty() {
                if let Ok(seq) = DnaSequence::from_str(
                    &current_seq,
                    "visualized".to_string(),
                    chunk_index,
                    current_seq.len() / 4,
                    0,
                ) {
                    sequences.push(seq);
                    chunk_index += 1;
                }
            }
            current_seq = String::new();
        } else {
            current_seq.push_str(line);
        }
    }

    if !current_seq.is_empty() {
        if let Ok(seq) = DnaSequence::from_str(
            &current_seq,
            "visualized".to_string(),
            chunk_index,
            current_seq.len() / 4,
            0,
        ) {
            sequences.push(seq);
        }
    }

    Ok(sequences)
}
