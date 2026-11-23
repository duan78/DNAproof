//! Lecture et écriture de fichiers FASTA.
//!
//! Les en-têtes sont purement informatifs : le décodage ne dépend que des séquences.
//!
//! La lecture tolère les fins de ligne CRLF, les lignes repliées, les minuscules, les lignes
//! vides et les blancs. Elle refuse les caractères qui ne sont pas des lettres (chiffres,
//! ponctuation). Les codes d'ambiguïté tels que `N` sont acceptés à la lecture ; les codecs les
//! écartent eux-mêmes et les comptent.

use crate::error::{DnaError, Result};
use crate::sequence::FASTA_LINE_WIDTH;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Marqueur des en-têtes du format de la version 0.1, qui n'est plus lisible.
pub const LEGACY_MARKER: &str = "|scheme:";

/// Message d'aide pour une archive de la version 0.1.
pub const LEGACY_HINT: &str = "cette archive utilise le format de la version 0.1, qui n'est plus \
     lu : ré-encoder le fichier original, ou le décoder avec le code du commit 09df4f8";

/// Un enregistrement FASTA : en-tête (sans le `>`) et séquence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub header: String,
    pub sequence: Vec<u8>,
}

/// Écrit des enregistrements au fil de l'eau, séquences repliées à 60 nucléotides par ligne.
///
/// L'archive est écrite dans un fichier temporaire voisin, puis renommée : un fichier
/// interrompu ne se présente jamais comme une archive complète.
pub struct Writer {
    file: BufWriter<File>,
    temporary: PathBuf,
    final_path: PathBuf,
}

impl Writer {
    /// Prépare l'écriture de l'archive `path`.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let final_path = path.as_ref().to_path_buf();
        let file_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                DnaError::Decoding(format!(
                    "chemin d'archive invalide: {}",
                    final_path.display()
                ))
            })?;
        let temporary = final_path.with_file_name(format!(".{file_name}.part"));
        let file = File::create(&temporary).map_err(|e| {
            DnaError::Decoding(format!("impossible de créer {}: {e}", temporary.display()))
        })?;
        Ok(Self {
            file: BufWriter::with_capacity(1 << 20, file),
            temporary,
            final_path,
        })
    }

    /// Écrit un enregistrement.
    pub fn write(&mut self, header: &str, sequence: &[u8]) -> Result<()> {
        self.file.write_all(b">")?;
        self.file.write_all(header.as_bytes())?;
        self.file.write_all(b"\n")?;
        for line in sequence.chunks(FASTA_LINE_WIDTH) {
            self.file.write_all(line)?;
            self.file.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Termine l'archive et la met en place.
    pub fn finish(self) -> Result<PathBuf> {
        let Self {
            mut file,
            temporary,
            final_path,
        } = self;
        file.flush()?;
        drop(file);
        std::fs::rename(&temporary, &final_path).map_err(|e| {
            DnaError::Io(std::io::Error::new(
                e.kind(),
                format!("{} -> {}: {e}", temporary.display(), final_path.display()),
            ))
        })?;
        Ok(final_path)
    }
}

/// Écrit des enregistrements dans un fichier, séquences repliées à 60 nucléotides par ligne.
pub fn write_file<P: AsRef<Path>>(path: P, records: &[Record]) -> Result<()> {
    let mut writer = Writer::create(path)?;
    for record in records {
        writer.write(&record.header, &record.sequence)?;
    }
    writer.finish()?;
    Ok(())
}

/// En-tête informatif d'un oligo d'archive de la version 2 : `adn-<schéma>-v2 manifest=<rang>`
/// pour les manifestes, qui viennent en tête, et `adn-<schéma>-v2 oligo=<rang>` pour les autres.
/// Chaque rang compte parmi les oligos de son espèce.
pub fn archive_header(
    scheme: crate::codec::Scheme,
    index: usize,
    manifest_copies: usize,
) -> String {
    let (kind, rank) = if index < manifest_copies {
        ("manifest", index)
    } else {
        ("oligo", index - manifest_copies)
    };
    format!("adn-{}-v2 {kind}={rank}", scheme.name())
}

/// Écrit une archive de la version 2, avec les en-têtes de [`archive_header`]. Séquences
/// repliées à 60 nucléotides par ligne. Rend le chemin de l'archive écrite.
pub fn write_archive<P: AsRef<Path>>(
    path: P,
    oligos: &[Vec<u8>],
    scheme: crate::codec::Scheme,
    manifest_copies: usize,
) -> Result<PathBuf> {
    let mut writer = Writer::create(path)?;
    for (index, oligo) in oligos.iter().enumerate() {
        writer.write(&archive_header(scheme, index, manifest_copies), oligo)?;
    }
    writer.finish()
}

/// Écrit un fichier d'un bloc, via un fichier temporaire voisin : en cas d'interruption, le
/// fichier visé reste absent ou inchangé, jamais à moitié écrit.
pub fn write_atomic<P: AsRef<Path>>(path: P, contents: &[u8]) -> Result<()> {
    let final_path = path.as_ref().to_path_buf();
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            DnaError::Decoding(format!(
                "chemin de fichier invalide: {}",
                final_path.display()
            ))
        })?;
    let temporary = final_path.with_file_name(format!(".{file_name}.part"));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, &final_path).map_err(|e| {
        DnaError::Io(std::io::Error::new(
            e.kind(),
            format!("{} -> {}: {e}", temporary.display(), final_path.display()),
        ))
    })?;
    Ok(())
}

/// Écrit un enregistrement sur un flux.
pub fn write_record<W: Write>(writer: &mut W, record: &Record) -> Result<()> {
    writer.write_all(b">")?;
    writer.write_all(record.header.as_bytes())?;
    writer.write_all(b"\n")?;
    for line in record.sequence.chunks(FASTA_LINE_WIDTH) {
        writer.write_all(line)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// Vrai si les en-têtes sont ceux du format de la version 0.1.
pub fn is_legacy_v1(records: &[Record]) -> bool {
    records
        .iter()
        .any(|record| record.header.contains(LEGACY_MARKER))
}

/// Lit un fichier FASTA.
pub fn read_file<P: AsRef<Path>>(path: P) -> Result<Vec<Record>> {
    let file = File::open(&path).map_err(|e| {
        DnaError::Decoding(format!(
            "impossible d'ouvrir {}: {e}",
            path.as_ref().display()
        ))
    })?;
    read_reader(BufReader::with_capacity(1 << 20, file))
}

/// Lit plusieurs fichiers FASTA et concatène leurs enregistrements.
pub fn read_files<P: AsRef<Path>>(paths: &[P]) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for path in paths {
        records.extend(read_file(path)?);
    }
    Ok(records)
}

fn unexpected(line: usize, byte: u8) -> DnaError {
    let shown = if byte.is_ascii_graphic() || byte == b' ' {
        format!("'{}'", byte as char)
    } else {
        format!("0x{byte:02X}")
    };
    DnaError::InvalidDna(format!(
        "ligne {line} : caractère {shown} inattendu dans une séquence"
    ))
}

/// Lit des enregistrements depuis un flux.
pub fn read_reader<R: BufRead>(reader: R) -> Result<Vec<Record>> {
    let mut records: Vec<Record> = Vec::new();
    let mut header: Option<String> = None;
    let mut sequence: Vec<u8> = Vec::new();
    let mut line_number = 0usize;

    for line in reader.split(b'\n') {
        let line = line.map_err(|e| DnaError::Decoding(format!("lecture impossible: {e}")))?;
        line_number += 1;
        let line = strip_eol(&line);

        match line.first() {
            None | Some(b';') => continue,
            Some(b'>') => {
                if let Some(previous) = header.take() {
                    records.push(Record {
                        header: previous,
                        sequence: std::mem::take(&mut sequence),
                    });
                }
                let text = String::from_utf8_lossy(&line[1..]).trim().to_string();
                if text.is_empty() {
                    return Err(DnaError::Decoding(format!(
                        "ligne {line_number} : en-tête vide"
                    )));
                }
                header = Some(text);
            }
            Some(_) => {
                if header.is_none() {
                    return Err(DnaError::Decoding(format!(
                        "ligne {line_number} : séquence avant le premier en-tête"
                    )));
                }
                for &byte in line {
                    match byte {
                        b'A'..=b'Z' => sequence.push(byte),
                        b'a'..=b'z' => sequence.push(byte.to_ascii_uppercase()),
                        b' ' | b'\t' => {}
                        _ => return Err(unexpected(line_number, byte)),
                    }
                }
            }
        }
    }
    if let Some(previous) = header.take() {
        records.push(Record {
            header: previous,
            sequence,
        });
    }
    Ok(records)
}

fn strip_eol(line: &[u8]) -> &[u8] {
    match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Vec<Record>> {
        read_reader(text.as_bytes())
    }

    #[test]
    fn tolerates_wrapping_crlf_lowercase_blanks_and_comments() {
        let text = "\r\n; commentaire\r\n>a desc\r\nacgt\r\nAATT\r\n\r\n>b\r\nGGCC\tTTAA\n";
        let records = parse(text).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].header, "a desc");
        assert_eq!(records[0].sequence, b"ACGTAATT");
        assert_eq!(records[1].sequence, b"GGCCTTAA");
    }

    #[test]
    fn accepts_ambiguity_codes_but_not_other_characters() {
        assert_eq!(parse(">x\nACGN\n").unwrap()[0].sequence, b"ACGN");
        let err = parse(">x\nACG1\n").unwrap_err();
        assert!(matches!(err, DnaError::InvalidDna(_)), "{err}");
        assert!(parse(">x\nACG-\n").is_err());
        assert!(parse("ACGT\n").is_err());
        assert!(parse(">\nACGT\n").is_err());
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn header_without_sequence_is_kept() {
        let records = parse(">empty\n>full\nACGT\n").unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[0].sequence.is_empty());
    }

    #[test]
    fn write_then_read_roundtrip() {
        let records = vec![
            Record {
                header: "un".to_string(),
                sequence: b"ACGT".repeat(100),
            },
            Record {
                header: "deux".to_string(),
                sequence: Vec::new(),
            },
        ];
        let dir = std::env::temp_dir().join(format!("adn_fasta_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.fasta");
        write_file(&path, &records).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(">un\n"));
        let longest = text
            .lines()
            .filter(|line| !line.starts_with('>'))
            .map(|line| line.len())
            .max()
            .unwrap_or(0);
        assert!(longest <= FASTA_LINE_WIDTH);
        assert_eq!(read_file(&path).unwrap(), records);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn legacy_headers_are_detected() {
        let records = parse(">id|scheme:fountain#4|seed:1\nACGT\n").unwrap();
        assert!(is_legacy_v1(&records));
        let records = parse(">adn-fountain-v2 seed=1\nACGT\n").unwrap();
        assert!(!is_legacy_v1(&records));
        assert!(read_file("fichier_inexistant.fasta").is_err());
    }

    #[test]
    fn archive_headers_count_manifests_and_oligos_separately() {
        assert_eq!(
            archive_header(crate::codec::Scheme::Fountain, 0, 2),
            "adn-fountain-v2 manifest=0"
        );
        assert_eq!(
            archive_header(crate::codec::Scheme::Fountain, 1, 2),
            "adn-fountain-v2 manifest=1"
        );
        assert_eq!(
            archive_header(crate::codec::Scheme::Strands, 2, 2),
            "adn-strands-v2 oligo=0"
        );
    }

    #[test]
    fn a_written_archive_reads_back() {
        let path =
            std::env::temp_dir().join(format!("adn-fasta-archive-{}.fa", std::process::id()));
        let oligos = vec![b"ACGT".to_vec(), b"TTTT".to_vec()];
        let written = write_archive(&path, &oligos, crate::codec::Scheme::Strands, 1).unwrap();
        let records = read_file(&written).unwrap();
        std::fs::remove_file(&written).unwrap();
        assert_eq!(records[0].header, "adn-strands-v2 manifest=0");
        assert_eq!(records[0].sequence, b"ACGT");
        assert_eq!(records[1].header, "adn-strands-v2 oligo=0");
        assert_eq!(records[1].sequence, b"TTTT");
    }
}
