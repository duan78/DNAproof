//! L'exemple de la bibliothèque donné dans le README, tel quel : il doit rester exact.

use adn_core::{decode, encode, DecodeOptions, EncodeOptions, Scheme};

#[test]
fn readme_example() {
    let directory = std::env::temp_dir().join(format!("adn-readme-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let archive_path = directory.join("archive.fasta");

    let options = EncodeOptions::for_scheme(Scheme::Strands);
    let archive = encode(b"donnees", &options).unwrap();
    adn_core::fasta::write_archive(
        &archive_path,
        &archive.oligos,
        Scheme::Strands,
        archive.stats.manifest_copies,
    )
    .unwrap();

    let records = adn_core::fasta::read_file(&archive_path).unwrap();
    let decoded = decode(
        records.iter().map(|record| record.sequence.as_slice()),
        &DecodeOptions::default(),
    )
    .unwrap();
    assert_eq!(decoded.data, b"donnees");

    std::fs::remove_dir_all(&directory).unwrap();
}
