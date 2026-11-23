//! `adn` : archive des fichiers en séquences d'ADN, et les restitue à l'identique.
//!
//! Une archive se décode depuis ses seules séquences : ni en-tête FASTA ni nom de fichier ne
//! sont nécessaires. Un fichier n'est jamais écrit si son empreinte SHA-256, embarquée dans
//! l'archive, ne correspond pas à ce qui a été reconstitué.

mod commands;
mod output;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use commands::{decode, encode, simulate, verify, visualize};

#[derive(Parser)]
#[command(
    name = "adn",
    version,
    about = "Archive des fichiers en séquences d'ADN, et les restitue à l'identique"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Sortie en JSON, sans mise en forme
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Encode un fichier en archive ADN (FASTA)
    Encode {
        /// Fichier à archiver
        #[arg(short, long)]
        input: PathBuf,

        /// Archive à écrire, ou répertoire où la placer (par défaut : à côté du fichier)
        #[arg(short, long)]
        output: Option<PathBuf>,

        #[command(flatten)]
        codec: CodecArgs,
    },

    /// Décode des lectures (un ou plusieurs fichiers FASTA) et restitue le fichier
    Decode {
        /// Fichiers de lectures ; plusieurs possibles
        #[arg(short, long = "input", num_args = 1..)]
        inputs: Vec<PathBuf>,

        /// Fichier à écrire (par défaut : <première entrée sans .fasta>.decoded)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Schéma attendu (par défaut : lu dans l'archive)
        #[arg(short, long, value_enum)]
        algorithm: Option<SchemeArg>,

        /// Empreinte SHA-256 attendue du fichier (64 chiffres hexadécimaux)
        #[arg(long)]
        expect_sha256: Option<String>,
    },

    /// Vérifie une archive : contraintes, manifeste, décodabilité ; n'écrit rien
    Verify {
        /// Archive à vérifier
        #[arg(short, long)]
        input: PathBuf,

        /// Contraintes à contrôler
        #[command(flatten)]
        limits: ConstraintArgs,

        /// Écrire aussi le fichier reconstitué
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Simule la dégradation d'une archive, puis tente de la relire
    Simulate {
        /// Fichier à archiver ; ou l'archive elle-même avec --archive
        #[arg(short, long)]
        input: PathBuf,

        /// L'entrée est une archive FASTA (le fichier d'origine n'est pas nécessaire)
        #[arg(long)]
        archive: bool,

        #[command(flatten)]
        codec: CodecArgs,

        #[command(flatten)]
        channel: ChannelArgs,

        /// Nombre de tirages
        #[arg(short = 'n', long, default_value_t = 10)]
        trials: usize,

        /// Graine du générateur
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },

    /// Statistiques d'un ensemble de séquences (table, JSON ou HTML)
    Visualize {
        /// Fichier FASTA
        #[arg(short, long)]
        input: PathBuf,

        /// Forme de la sortie
        #[arg(short, long, value_enum, default_value_t = FormatArg::Table)]
        format: FormatArg,

        /// Fichier de sortie (par défaut : la sortie standard)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Contraintes à contrôler
        #[command(flatten)]
        limits: ConstraintArgs,

        /// Toutes les séquences (par défaut : les 30 premières en tableau)
        #[arg(long)]
        all: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SchemeArg {
    /// DNA Fountain (Erlich et Zielinski, 2017)
    Fountain,
    /// Brins codés, Reed-Solomon interne et externe
    Strands,
}

impl SchemeArg {
    fn scheme(self) -> adn_core::Scheme {
        match self {
            SchemeArg::Fountain => adn_core::Scheme::Fountain,
            SchemeArg::Strands => adn_core::Scheme::Strands,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CompressionArg {
    /// zstd s'il rapporte, sinon aucune
    Auto,
    /// Aucune
    None,
    Lz4,
    Zstd,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    Table,
    Json,
    Html,
}

/// Contraintes biochimiques, celles de l'article DNA Fountain par défaut.
#[derive(clap::Args)]
struct ConstraintArgs {
    /// Part minimale de G et C
    #[arg(long, default_value_t = 0.45, value_name = "0-1")]
    gc_min: f64,

    /// Part maximale de G et C
    #[arg(long, default_value_t = 0.55, value_name = "0-1")]
    gc_max: f64,

    /// Longueur maximale des suites de bases identiques
    #[arg(long, default_value_t = 3)]
    max_homopolymer: usize,
}

impl ConstraintArgs {
    fn constraints(&self) -> anyhow::Result<adn_core::DnaConstraints> {
        let constraints =
            adn_core::DnaConstraints::new(self.gc_min, self.gc_max, self.max_homopolymer);
        constraints.validate()?;
        Ok(constraints)
    }
}

/// Paramètres d'encodage.
#[derive(clap::Args)]
struct CodecArgs {
    /// Schéma d'encodage
    #[arg(short, long, value_enum, default_value_t = SchemeArg::Fountain)]
    algorithm: SchemeArg,

    /// Marge de perte (DNA Fountain) : r signifie que l'archive reste lisible après la perte
    /// aléatoire d'une fraction 1 - 1/r des oligos
    #[arg(short, long, default_value_t = 1.1, value_name = ">=1")]
    redundancy: f64,

    /// Octets de parité Reed-Solomon par oligo (DNA Fountain) : un oligo résiste à la moitié
    /// d'octets erronés, et s'allonge de 4 nt par octet. Utile devant des erreurs de synthèse,
    /// que toutes les lectures d'un oligo portent à l'identique
    #[arg(long, default_value_t = 2, value_name = "1-32", value_parser = clap::value_parser!(u8).range(1..=32))]
    rs_parity: u8,

    /// Compression du fichier avant encodage
    #[arg(short = 'c', long, value_enum, default_value_t = CompressionArg::Auto)]
    compression: CompressionArg,

    #[command(flatten)]
    limits: ConstraintArgs,
}

impl CodecArgs {
    fn options(&self) -> anyhow::Result<adn_core::EncodeOptions> {
        let compression = match self.compression {
            CompressionArg::Auto => adn_core::Compression::Auto,
            CompressionArg::None => adn_core::Compression::None,
            CompressionArg::Lz4 => adn_core::Compression::Lz4,
            CompressionArg::Zstd => adn_core::Compression::Zstd,
        };
        let options = adn_core::EncodeOptions {
            scheme: self.algorithm.scheme(),
            compression,
            fountain: adn_core::codec::fountain::FountainParams {
                redundancy: self.redundancy,
                rs_parity: usize::from(self.rs_parity),
                ..adn_core::codec::fountain::FountainParams::default()
            },
            constraints: self.limits.constraints()?,
        };
        options.validate()?;
        Ok(options)
    }
}

/// Paramètres du canal simulé. Les taux sont par nucléotide.
#[derive(clap::Args)]
struct ChannelArgs {
    /// Taux de substitution du séquençage
    #[arg(long, default_value_t = 0.001, value_name = "TAUX")]
    substitution_rate: f64,

    /// Taux d'insertion du séquençage
    #[arg(long, default_value_t = 0.0, value_name = "TAUX")]
    insertion_rate: f64,

    /// Taux de délétion du séquençage
    #[arg(long, default_value_t = 0.0, value_name = "TAUX")]
    deletion_rate: f64,

    /// Taux de substitution de la synthèse, porté par toutes les lectures d'un oligo
    #[arg(long, default_value_t = 0.0, value_name = "TAUX")]
    synthesis_substitution_rate: f64,

    /// Taux de délétion de la synthèse, porté par toutes les lectures d'un oligo
    #[arg(long, default_value_t = 0.0, value_name = "TAUX")]
    synthesis_deletion_rate: f64,

    /// Taux d'insertion de la synthèse, porté par toutes les lectures d'un oligo
    #[arg(long, default_value_t = 0.0, value_name = "TAUX")]
    synthesis_insertion_rate: f64,

    /// Fraction des oligos absents de la bibliothèque
    #[arg(long, default_value_t = 0.0, value_name = "0-1")]
    dropout: f64,

    /// Nombre moyen de lectures par oligo
    #[arg(long, default_value_t = 1.0, value_name = "MOYENNE")]
    coverage: f64,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let out = output::Printer::new(cli.json);
    match cli.command {
        Command::Encode {
            input,
            output,
            codec,
        } => encode::run(&input, output, &codec, &out),
        Command::Decode {
            inputs,
            output,
            algorithm,
            expect_sha256,
        } => decode::run(&inputs, output, algorithm, expect_sha256.as_deref(), &out),
        Command::Verify {
            input,
            limits,
            output,
        } => verify::run(&input, &limits, output, &out),
        Command::Simulate {
            input,
            archive,
            codec,
            channel,
            trials,
            seed,
        } => simulate::run(&input, archive, &codec, &channel, trials, seed, &out),
        Command::Visualize {
            input,
            format,
            output,
            limits,
            all,
        } => visualize::run(&input, format, output, &limits, all, &out),
    }
}
