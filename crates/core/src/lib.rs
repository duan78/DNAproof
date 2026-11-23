//! adn-core : codecs d'archivage de données numériques en séquences d'ADN.
//!
//! Quatre schémas produisent des oligos qui se suffisent à eux-mêmes : une archive se décode
//! depuis ses seules séquences, et un fichier n'est jamais rendu si son empreinte SHA-256 n'est
//! pas vérifiée.
//!
//! - [`Scheme::Fountain`] : DNA Fountain (Erlich et Zielinski, 2017).
//! - [`Scheme::Strands`] : brins codés, à deux niveaux de Reed-Solomon.
//! - [`Scheme::Goldman`] : code ternaire de Huffman, sans base répétée, fenêtres recouvrantes
//!   (Goldman et al., 2013).
//! - [`Scheme::Grass`] : Reed-Solomon sur le corps à 47 éléments, un symbole par trimère
//!   (Grass et al., 2015).
//!
//! ```
//! let options = adn_core::EncodeOptions::default();
//! let archive = adn_core::encode(b"donnees", &options).unwrap();
//! let decoded = adn_core::decode(&archive.oligos, &adn_core::DecodeOptions::default()).unwrap();
//! assert_eq!(decoded.data, b"donnees");
//! ```
//!
//! Le format est décrit dans `docs/FORMAT.md`.

pub mod checksum;
pub mod codec;
pub mod constraints;
pub mod envelope;
pub mod error;
pub mod fasta;
pub mod rs;
pub mod rs47;
pub mod sequence;
pub mod verify;

pub use codec::{
    decode, encode, DecodeOptions, DecodeStats, Decoded, EncodeOptions, EncodeStats, Encoded,
    Scheme,
};
pub use constraints::DnaConstraints;
pub use envelope::Compression;
pub use error::{DnaError, Result};
pub use fasta::Record;
pub use verify::{OligoReport, PoolReport};
