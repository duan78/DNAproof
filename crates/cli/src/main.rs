//! CLI pour l'application ADN

use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

mod commands;
mod display;

use commands::{encode, decode, simulate, visualize};

#[derive(Parser)]
#[command(name = "adn")]
#[command(about = "Encodage de fichiers en ADN virtuel", long_about = None)]
#[command(version = "0.1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Niveau de verbosité
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Subcommand)]
enum Commands {
    /// Encode un fichier en séquences ADN
    Encode {
        /// Fichier d'entrée
        #[arg(short, long)]
        input: PathBuf,

        /// Répertoire de sortie
        #[arg(short, long)]
        output: PathBuf,

        /// Algorithme d'encodage
        #[arg(short, long, value_enum)]
        algorithm: EncodingAlgorithm,

        /// Facteur de redondance (1.0 = pas de redondance)
        #[arg(short, long, default_value = "1.5")]
        redundancy: f64,

        /// Activer la compression
        #[arg(short = 'z', long, default_value = "true")]
        compress: bool,

        /// Algorithme de compression
        #[arg(short = 'c', long, value_enum)]
        compression: Option<CompressionAlgorithm>,
    },

    /// Décode des séquences ADN en fichier original
    Decode {
        /// Fichier FASTA/JSON d'entrée
        #[arg(short, long)]
        input: PathBuf,

        /// Fichier de sortie
        #[arg(short, long)]
        output: PathBuf,

        /// Ignorer les erreurs de checksum
        #[arg(short, long)]
        ignore_checksum: bool,
    },

    /// Simule des erreurs de stockage ADN
    Simulate {
        /// Fichier de séquences
        #[arg(short, long)]
        input: PathBuf,

        /// Taux d'erreur de substitution (0.0-1.0)
        #[arg(short, long, default_value = "0.01")]
        substitution_rate: f64,

        /// Taux d'erreur d'insertion (0.0-1.0)
        #[arg(short = 'i', long, default_value = "0.005")]
        insertion_rate: f64,

        /// Taux d'erreur de délétion (0.0-1.0)
        #[arg(short = 'd', long, default_value = "0.005")]
        deletion_rate: f64,

        /// Nombre d'itérations
        #[arg(short = 'n', long, default_value = "100")]
        iterations: usize,
    },

    /// Visualise les statistiques et métadonnées
    Visualize {
        /// Fichier de séquences
        #[arg(short, long)]
        input: PathBuf,

        /// Type de visualisation
        #[arg(short, long, value_enum)]
        format: VisualizationFormat,

        /// Exporter en fichier
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(clap::ValueEnum, Clone)]
pub enum EncodingAlgorithm {
    Fountain,
    Goldman,
    Goldman2013,
    Grass2015,
    Adaptive,
    Base3,
}

#[derive(clap::ValueEnum, Clone)]
pub enum CompressionAlgorithm {
    Lz4,
    Zstd,
    None,
}

#[derive(clap::ValueEnum, Clone)]
pub enum VisualizationFormat {
    Table,
    Json,
    Html,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Encode {
            input,
            output,
            algorithm,
            redundancy,
            compress,
            compression,
        } => {
            encode::run(input, output, algorithm, redundancy, compress, compression)?;
        }
        Commands::Decode {
            input,
            output,
            ignore_checksum,
        } => {
            decode::run(input, output, ignore_checksum)?;
        }
        Commands::Simulate {
            input,
            substitution_rate,
            insertion_rate,
            deletion_rate,
            iterations,
        } => {
            simulate::run(input, substitution_rate, insertion_rate, deletion_rate, iterations)?;
        }
        Commands::Visualize {
            input,
            format,
            output,
        } => {
            visualize::run(input, format, output)?;
        }
    }

    Ok(())
}

/// Crée une barre de progression
pub fn create_progress_bar(length: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(length);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("##-"),
    );
    pb.set_message(msg.to_string());
    pb
}

/// Crée une barre de progression spinner
pub fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(ProgressStyle::default_spinner()
        .template("{spinner:.green} [{elapsed_precise}] {msg}")
        .unwrap());
    pb.set_message(msg.to_string());
    pb
}
