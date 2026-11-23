//! Travail d'une tâche : encoder un fichier en archive, ou décoder une archive en fichier.
//!
//! Les deux fonctions sont synchrones et bloquantes : le serveur les mène sur ses threads de
//! travail (voir [`crate::jobs::launch`]).

use crate::jobs::{JobKind, JobRegistry, Outcome};
use adn_core::codec::fountain::FountainParams;
use adn_core::{DecodeOptions, EncodeOptions, Scheme};
use adn_storage::ArchiveInfo;
use std::io::Cursor;
use std::time::Instant;

/// Nom proposé quand l'archive n'en laisse aucun deviner.
const RESTORED_FILE_NAME: &str = "fichier-restaure";

/// Nom donné au fichier d'origine quand le navigateur n'en transmet pas.
pub const UNNAMED_FILE: &str = "fichier";

/// Fichier à encoder.
#[derive(Debug)]
pub struct EncodeRequest {
    /// Nom du fichier envoyé, sans chemin.
    pub source_name: String,
    pub data: Vec<u8>,
    pub options: EncodeOptions,
}

/// Archive à décoder.
#[derive(Debug)]
pub struct DecodeRequest {
    /// Nom de l'archive envoyée, sans chemin.
    pub source_name: String,
    pub data: Vec<u8>,
    /// Empreinte SHA-256 que doit avoir le fichier restitué.
    pub expected_sha256: Option<[u8; 32]>,
}

/// Encode un fichier et écrit l'archive. Son empreinte SHA-256, embarquée dans l'archive,
/// est rendue avec le résultat.
pub fn encode(
    registry: &JobRegistry,
    job_id: &str,
    request: EncodeRequest,
) -> Result<Outcome, String> {
    let EncodeRequest {
        source_name,
        data,
        options,
    } = request;

    registry.set_stage(job_id, "encodage");
    let started = Instant::now();
    let encoded = adn_core::encode(&data, &options)
        .map_err(|error| format!("encodage impossible : {error}"))?;
    let encode_seconds = started.elapsed().as_secs_f64();
    let stats = &encoded.stats;
    let sha256 = adn_core::checksum::hex(&stats.sha256);
    let oligo_length = options.oligo_length();

    registry.set_stage(job_id, "écriture de l'archive");
    let path = registry.result_path(job_id, JobKind::Encode);
    adn_core::fasta::write_archive(
        &path,
        &encoded.oligos,
        options.scheme,
        stats.manifest_copies,
    )
    .map_err(|error| format!("impossible d'écrire l'archive : {error}"))?;
    let size = std::fs::metadata(&path)
        .map_err(|error| format!("taille de l'archive : {error}"))?
        .len();
    let measures = measures(
        stats,
        oligo_length,
        &options,
        encode_seconds,
        size,
        registry.database().is_some(),
    );

    if registry.database().is_some() {
        registry.set_stage(job_id, "archivage en base de données");
        let info = ArchiveInfo {
            id: job_id.to_string(),
            file_name: source_name.clone(),
            sha256: sha256.clone(),
            scheme: options.scheme.name().to_string(),
            stats: measures.to_string(),
        };
        registry.store_archive_blocking(&info, &encoded.oligos)?;
    }

    Ok(Outcome {
        file_name: archive_download_name(&source_name),
        sha256,
        size,
        stats: measures,
    })
}

/// Mesures rendues avec une archive encodée.
fn measures(
    stats: &adn_core::EncodeStats,
    oligo_length: usize,
    options: &EncodeOptions,
    encode_seconds: f64,
    size: u64,
    stored: bool,
) -> serde_json::Value {
    let mut measures = serde_json::json!({
        "scheme": options.scheme,
        "compression": stats.compression,
        "original_bytes": stats.original_len,
        "body_bytes": stats.body_len,
        "units": stats.units,
        "oligos": stats.data_oligos + stats.manifest_copies,
        "data_oligos": stats.data_oligos,
        "manifest_copies": stats.manifest_copies,
        "oligo_length_nt": oligo_length,
        "nucleotides": stats.nucleotides,
        "bits_per_nucleotide": stats.bits_per_nucleotide,
        "droplets_per_unit": stats.data_oligos as f64 / f64::from(stats.units.max(1)),
        "candidates_tried": stats.candidates_tried,
        "encode_seconds": encode_seconds,
        "archive_bytes": size,
        "stored_in_database": stored,
    });
    if options.scheme == Scheme::Fountain {
        measures["redundancy"] = serde_json::json!(options.fountain.redundancy);
    }
    measures
}

/// Lit une archive et écrit le fichier qu'elle contient. Le fichier n'est écrit que si son
/// empreinte SHA-256, embarquée dans l'archive, est vérifiée.
pub fn decode(
    registry: &JobRegistry,
    job_id: &str,
    request: DecodeRequest,
) -> Result<Outcome, String> {
    let DecodeRequest {
        source_name,
        data,
        expected_sha256,
    } = request;

    registry.set_stage(job_id, "lecture des séquences");
    let records = adn_core::fasta::read_reader(Cursor::new(&data))
        .map_err(|error| format!("archive illisible : {error}"))?;
    if adn_core::fasta::is_legacy_v1(&records) {
        return Err(format!(
            "archive au format de la version 0.1 : {}",
            adn_core::fasta::LEGACY_HINT
        ));
    }
    if records.is_empty() {
        return Err("l'archive ne contient aucune séquence".to_string());
    }

    registry.set_stage(job_id, "décodage");
    let started = Instant::now();
    let reads: Vec<Vec<u8>> = records.into_iter().map(|record| record.sequence).collect();
    let decoded = adn_core::decode(reads, &DecodeOptions::default())
        .map_err(|error| format!("décodage impossible : {error}"))?;
    let decode_seconds = started.elapsed().as_secs_f64();
    let sha256 = adn_core::checksum::hex(&decoded.sha256);

    if let Some(expected) = expected_sha256 {
        if expected != decoded.sha256 {
            return Err(format!(
                "empreinte inattendue : le fichier reconstitué a pour SHA-256 {sha256}"
            ));
        }
    }

    registry.set_stage(job_id, "écriture du fichier");
    let path = registry.result_path(job_id, JobKind::Decode);
    adn_core::fasta::write_atomic(&path, &decoded.data)
        .map_err(|error| format!("impossible d'écrire le fichier : {error}"))?;

    Ok(Outcome {
        file_name: decoded_file_name(&source_name),
        sha256,
        size: decoded.data.len() as u64,
        stats: {
            let mut stats =
                serde_json::to_value(&decoded.stats).unwrap_or_else(|_| serde_json::json!({}));
            for (key, value) in [
                ("scheme", serde_json::json!(decoded.scheme)),
                ("compression", serde_json::json!(decoded.compression)),
                ("decode_seconds", serde_json::json!(decode_seconds)),
                ("file_bytes", serde_json::json!(decoded.data.len())),
            ] {
                stats[key] = value;
            }
            stats
        },
    })
}

/// Nom proposé pour l'archive : celui du fichier d'origine, suivi de `.fasta`. Un fichier qui
/// est déjà une archive en reçoit un suffixe qui distingue la nouvelle.
pub fn archive_download_name(original: &str) -> String {
    if archive_stem(&original.to_ascii_lowercase()).is_some() {
        format!("{original}.adn.fasta")
    } else {
        format!("{original}.fasta")
    }
}

/// Nom proposé pour le fichier restitué : celui de l'archive, sans son suffixe.
pub fn decoded_file_name(archive_name: &str) -> String {
    // La casse ne change pas la longueur : la longueur du radical vaut pour les deux.
    match archive_stem(&archive_name.to_ascii_lowercase()) {
        Some(stem) if !stem.trim().is_empty() => archive_name[..stem.len()].trim().to_string(),
        _ => RESTORED_FILE_NAME.to_string(),
    }
}

/// Nom sans son suffixe d'archive (`.fasta`, `.fas`, `.fa`), s'il en a un.
fn archive_stem(lower_name: &str) -> Option<&str> {
    [".fasta", ".fas", ".fa"]
        .iter()
        .find_map(|suffix| lower_name.strip_suffix(suffix))
}

/// Redondance proposée par défaut pour DNA Fountain.
pub fn default_redundancy() -> f64 {
    FountainParams::default().redundancy
}

/// Borne inférieure de la marge de perte proposée par la page d'encodage. Une marge de 1.0 ne
/// laisse rien pour survivre à la moindre perte.
pub const MIN_REDUNDANCY: f64 = 1.05;
const _: () = assert!(MIN_REDUNDANCY > 1.0);

/// Borne supérieure de la marge de perte proposée par la page d'encodage.
pub const MAX_REDUNDANCY: f64 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_margin_still_survives_losses() {
        // La fraction d'oligos que l'archive peut perdre : 1 − 1 / marge.
        let loss = 1.0 - 1.0 / MIN_REDUNDANCY;
        assert!(loss > 0.04 && loss < 0.05);
        let default = default_redundancy();
        assert!((MIN_REDUNDANCY..=MAX_REDUNDANCY).contains(&default));
    }

    #[test]
    fn archive_names_follow_the_original_file() {
        assert_eq!(archive_download_name("lettre.txt"), "lettre.txt.fasta");
        assert_eq!(archive_download_name("archive"), "archive.fasta");
        assert_eq!(archive_download_name("photo.jpg"), "photo.jpg.fasta");
        assert_eq!(archive_download_name("données.csv"), "données.csv.fasta");
        // Archiver une archive ne doit pas donner deux fois le même nom.
        assert_eq!(
            archive_download_name("données.fasta"),
            "données.fasta.adn.fasta"
        );
        assert_eq!(archive_download_name("DONNEES.FA"), "DONNEES.FA.adn.fasta");
    }

    #[test]
    fn restored_names_lose_the_archive_suffix() {
        assert_eq!(decoded_file_name("lettre.txt.fasta"), "lettre.txt");
        assert_eq!(decoded_file_name("photo.jpg.FASTA"), "photo.jpg");
        assert_eq!(decoded_file_name("archive.fa"), "archive");
        assert_eq!(decoded_file_name("archive.fas"), "archive");
        assert_eq!(decoded_file_name("sans-suffixe"), RESTORED_FILE_NAME);
        assert_eq!(decoded_file_name(".fasta"), RESTORED_FILE_NAME);
        assert_eq!(
            decoded_file_name("données.fasta.adn.fasta"),
            "données.fasta.adn"
        );
    }
}
