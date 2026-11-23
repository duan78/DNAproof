//! Lecture et écriture de fichiers FASTA et FASTQ.
//!
//! Les en-têtes sont purement informatifs : le décodage ne dépend que des séquences.
//!
//! La lecture reconnaît le format à son premier caractère utile : `>` ouvre un FASTA, `@` un
//! FASTQ. Elle tolère les fins de ligne CRLF, les lignes repliées (FASTA), les minuscules, les
//! lignes vides et les blancs. Elle refuse les caractères qui ne sont pas des lettres (chiffres,
//! ponctuation). Les codes d'ambiguïté tels que `N` sont acceptés à la lecture ; les codecs les
//! écartent eux-mêmes et les comptent. Les qualités d'un FASTQ sont contrôlées par leur
//! longueur, puis ignorées : les décodeurs ne les utilisent pas.

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

/// Fichier en cours d'écriture, via un fichier temporaire voisin : en cas d'interruption, le
/// fichier visé reste absent ou inchangé, jamais à moitié écrit.
struct StagedFile {
    file: BufWriter<File>,
    temporary: PathBuf,
    final_path: PathBuf,
}

impl StagedFile {
    fn create(path: &Path) -> Result<Self> {
        let final_path = path.to_path_buf();
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

    fn finish(self) -> Result<PathBuf> {
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

/// Écrit des enregistrements FASTA au fil de l'eau, séquences repliées à 60 nucléotides par
/// ligne.
///
/// L'archive est écrite dans un fichier temporaire voisin, puis renommée : un fichier
/// interrompu ne se présente jamais comme une archive complète.
pub struct Writer {
    staged: StagedFile,
}

impl Writer {
    /// Prépare l'écriture de l'archive `path`.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self {
            staged: StagedFile::create(path.as_ref())?,
        })
    }

    /// Écrit un enregistrement.
    pub fn write(&mut self, header: &str, sequence: &[u8]) -> Result<()> {
        let file = &mut self.staged.file;
        file.write_all(b">")?;
        file.write_all(header.as_bytes())?;
        file.write_all(b"\n")?;
        for line in sequence.chunks(FASTA_LINE_WIDTH) {
            file.write_all(line)?;
            file.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Termine l'archive et la met en place.
    pub fn finish(self) -> Result<PathBuf> {
        self.staged.finish()
    }
}

/// Écrit des enregistrements FASTQ au fil de l'eau : un enregistrement par quatre lignes,
/// qualités Phred+33, un caractère par base. Comme [`Writer`], le fichier n'apparaît entier
/// qu'une fois terminé.
pub struct FastqWriter {
    staged: StagedFile,
}

impl FastqWriter {
    /// Prépare l'écriture du fichier `path`.
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self {
            staged: StagedFile::create(path.as_ref())?,
        })
    }

    /// Écrit un enregistrement. Refuse un en-tête ou des qualités qui tiendraient sur plusieurs
    /// lignes, et des qualités plus courtes ou plus longues que la séquence.
    pub fn write(&mut self, header: &str, sequence: &[u8], quality: &[u8]) -> Result<()> {
        let broken = |what: &str| DnaError::Encoding(format!("en-tête « {header} » : {what}"));
        if header.is_empty() || header.contains(['\n', '\r']) {
            return Err(broken("un en-tête tient sur une ligne non vide"));
        }
        if quality.len() != sequence.len() {
            return Err(broken(&format!(
                "{} qualités pour {} bases",
                quality.len(),
                sequence.len()
            )));
        }
        if let Some(&byte) = quality.iter().find(|&&byte| !(33..=126).contains(&byte)) {
            return Err(broken(&format!(
                "qualité {byte} hors de l'échelle Phred+33 (33 à 126)"
            )));
        }
        let file = &mut self.staged.file;
        file.write_all(b"@")?;
        file.write_all(header.as_bytes())?;
        file.write_all(b"\n")?;
        file.write_all(sequence)?;
        file.write_all(b"\n+\n")?;
        file.write_all(quality)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    /// Termine le fichier et le met en place.
    pub fn finish(self) -> Result<PathBuf> {
        self.staged.finish()
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
    let mut staged = StagedFile::create(path.as_ref())?;
    staged.file.write_all(contents)?;
    staged.finish()?;
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

/// Lit un fichier FASTA ou FASTQ.
pub fn read_file<P: AsRef<Path>>(path: P) -> Result<Vec<Record>> {
    let file = File::open(&path).map_err(|e| {
        DnaError::Decoding(format!(
            "impossible d'ouvrir {}: {e}",
            path.as_ref().display()
        ))
    })?;
    read_reader(BufReader::with_capacity(1 << 20, file))
}

/// Lit plusieurs fichiers FASTA ou FASTQ et concatène leurs enregistrements.
pub fn read_files<P: AsRef<Path>>(paths: &[P]) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for path in paths {
        records.extend(read_file(path)?);
    }
    Ok(records)
}

fn unreadable(e: std::io::Error) -> DnaError {
    DnaError::Decoding(format!("lecture impossible: {e}"))
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

/// Lit des enregistrements depuis un flux FASTA ou FASTQ.
///
/// Le format se reconnaît à son premier caractère utile : `@` ouvre un FASTQ, lu par blocs de
/// quatre lignes ; tout autre ouvre un FASTA.
pub fn read_reader<R: BufRead>(mut reader: R) -> Result<Vec<Record>> {
    let mut line_number = 0usize;
    if opens_fastq(&mut reader, &mut line_number)? {
        read_fastq(&mut reader, line_number)
    } else {
        read_fasta(&mut reader, line_number)
    }
}

/// Saute les blancs du début du fichier et dit s'il ouvre un FASTQ.
fn opens_fastq<R: BufRead>(reader: &mut R, line_number: &mut usize) -> Result<bool> {
    loop {
        let buffer = reader.fill_buf().map_err(unreadable)?;
        match buffer.first() {
            None => return Ok(false),
            Some(b'\n') => {
                *line_number += 1;
                reader.consume(1);
            }
            Some(b'\r') | Some(b' ') | Some(b'\t') => reader.consume(1),
            Some(b'@') => return Ok(true),
            Some(_) => return Ok(false),
        }
    }
}

/// Une ligne d'un fichier FASTQ, fin de ligne retirée ; `None` à la fin du fichier.
fn next_line<R: BufRead>(reader: &mut R, line_number: &mut usize) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    if reader.read_until(b'\n', &mut line).map_err(unreadable)? == 0 {
        return Ok(None);
    }
    *line_number += 1;
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    Ok(Some(line))
}

fn parse_sequence(line: &[u8], line_number: usize) -> Result<Vec<u8>> {
    let mut sequence = Vec::with_capacity(line.len());
    for &byte in line {
        match byte {
            b'A'..=b'Z' => sequence.push(byte),
            b'a'..=b'z' => sequence.push(byte.to_ascii_uppercase()),
            b' ' | b'\t' => {}
            _ => return Err(unexpected(line_number, byte)),
        }
    }
    Ok(sequence)
}

fn read_fasta<R: BufRead>(reader: R, mut line_number: usize) -> Result<Vec<Record>> {
    let mut records: Vec<Record> = Vec::new();
    let mut header: Option<String> = None;
    let mut sequence: Vec<u8> = Vec::new();

    for line in reader.split(b'\n') {
        let line = line.map_err(unreadable)?;
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
                sequence.extend_from_slice(&parse_sequence(line, line_number)?);
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

fn read_fastq<R: BufRead>(mut reader: R, mut line_number: usize) -> Result<Vec<Record>> {
    let incomplete = |line_number: usize| {
        DnaError::Decoding(format!(
            "ligne {line_number} : enregistrement FASTQ incomplet (en-tête, séquence, « + », \
             qualités)"
        ))
    };
    let mut records = Vec::new();
    while let Some(line) = next_line(&mut reader, &mut line_number)? {
        if line.is_empty() {
            continue;
        }
        if line[0] != b'@' {
            return Err(DnaError::Decoding(format!(
                "ligne {line_number} : en-tête FASTQ attendu, qui commence par @"
            )));
        }
        let header = String::from_utf8_lossy(&line[1..]).trim().to_string();
        if header.is_empty() {
            return Err(DnaError::Decoding(format!(
                "ligne {line_number} : en-tête vide"
            )));
        }
        let sequence = match next_line(&mut reader, &mut line_number)? {
            Some(line) => parse_sequence(&line, line_number)?,
            None => return Err(incomplete(line_number)),
        };
        match next_line(&mut reader, &mut line_number)? {
            Some(line) if line.first() == Some(&b'+') => {}
            Some(_) => {
                return Err(DnaError::Decoding(format!(
                    "ligne {line_number} : ligne « + » attendue après la séquence"
                )));
            }
            None => return Err(incomplete(line_number)),
        }
        // Les qualités comptent un caractère par base : c'est aussi ce qui garantit qu'elles
        // tiennent sur une seule ligne, quand bien même elles commenceraient par `@` ou `+`.
        match next_line(&mut reader, &mut line_number)? {
            Some(quality) if quality.len() == sequence.len() => {}
            Some(quality) => {
                return Err(DnaError::Decoding(format!(
                    "ligne {line_number} : {} caractères de qualité pour une séquence de {} bases",
                    quality.len(),
                    sequence.len()
                )));
            }
            None => return Err(incomplete(line_number)),
        }
        records.push(Record { header, sequence });
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
    fn fastq_is_recognized_and_read_by_blocks_of_four_lines() {
        let text = "@r1 desc 1:N:0:ATCG\nACGTACGT\n+\nIIIIIIII\n@r2\nacgt\n+r2 desc\nHHHH\n";
        let records = parse(text).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].header, "r1 desc 1:N:0:ATCG");
        assert_eq!(records[0].sequence, b"ACGTACGT");
        assert_eq!(records[1].header, "r2");
        assert_eq!(records[1].sequence, b"ACGT");
        // Des fins de ligne CRLF ne comptent pas comme des caractères de qualité.
        let records = parse("@r1\r\nACGT\r\n+\r\nIIII\r\n").unwrap();
        assert_eq!(records[0].sequence, b"ACGT");
        // Une qualité peut commencer par `@` ou `+` : seule la longueur compte, et elle tient
        // sur une ligne.
        let records = parse("@r1\nACGT\n+\n@@@@\n@r2\nTTTT\n+\n++++\n").unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].sequence, b"TTTT");
        // Lignes vides entre deux enregistrements, blancs en tête, séquence repliée non.
        let records = parse("\n \n@r1\nAC GT\n+\nIIII\n\n").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].sequence, b"ACGT");
        // Un FASTA peut garder ses lignes vides en tête et ses commentaires.
        let records = parse("\n\n; note\n>a\nACGT\n").unwrap();
        assert_eq!(records[0].header, "a");
    }

    #[test]
    fn damaged_fastq_is_refused_with_its_line() {
        for (text, expected) in [
            // Qualités trop courtes.
            (
                "@r1\nACGT\n+\nIII\n",
                "ligne 4 : 3 caractères de qualité pour une séquence de 4 bases",
            ),
            // Qualités trop longues.
            ("@r1\nACGT\n+\nIIIII\n", "5 caractères de qualité"),
            // Enregistrement coupé après chaque ligne.
            ("@r1\n", "ligne 1 : enregistrement FASTQ incomplet"),
            ("@r1\nACGT\n", "ligne 2 : enregistrement FASTQ incomplet"),
            ("@r1\nACGT\n+\n", "ligne 3 : enregistrement FASTQ incomplet"),
            // Ligne « + » manquante.
            ("@r1\nACGT\nIIII\n", "ligne 3 : ligne « + » attendue"),
            // En-tête vide, ou absolu.
            ("@\nACGT\n+\nIIII\n", "ligne 1 : en-tête vide"),
            (
                "@r1\nACGT\n+\nIIII\nr2\n",
                "ligne 5 : en-tête FASTQ attendu",
            ),
            // Caractère interdit dans la séquence.
            ("@r1\nACG1\n+\nIIII\n", "ligne 2 : caractère '1'"),
        ] {
            let err = parse(text).unwrap_err();
            assert!(err.to_string().contains(expected), "{text:?} : {err}");
        }
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
    fn fastq_is_written_one_record_per_four_lines() {
        let dir = std::env::temp_dir().join(format!("adn_fastq_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("reads.fastq");
        let mut writer = FastqWriter::create(&path).unwrap();
        writer
            .write("r1 1:N:0:TAAGGCGA", b"ACGTAC", b"IIFF#A")
            .unwrap();
        writer.write("r2", b"", b"").unwrap();
        let written = writer.finish().unwrap();
        assert_eq!(
            std::fs::read_to_string(&written).unwrap(),
            "@r1 1:N:0:TAAGGCGA\nACGTAC\n+\nIIFF#A\n@r2\n\n+\n\n"
        );
        let records = read_file(&written).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].sequence, b"ACGTAC");
        assert_eq!(records[1].sequence, b"");
        std::fs::remove_file(&written).unwrap();

        // Ce qui casserait le fichier est refusé.
        let mut writer = FastqWriter::create(&path).unwrap();
        assert!(writer.write("r1", b"ACGT", b"III").is_err());
        assert!(writer.write("r1", b"ACGT", b"IIIII").is_err());
        assert!(writer.write("r1\nr2", b"ACGT", b"IIII").is_err());
        assert!(writer.write("", b"ACGT", b"IIII").is_err());
        assert!(writer.write("r1", b"ACGT", b"I\nII").is_err());
        assert!(writer.write("r1", b"ACGT", b"I\x7FII").is_err());
        assert!(writer.write("r1", b"ACGT", b"I III").is_err());
        writer.write("r1", b"ACGT", &[33u8; 4]).unwrap();
        writer.finish().unwrap();
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
