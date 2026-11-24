//! `adn` : archive des fichiers en séquences d'ADN, et les restitue à l'identique.
//!
//! Une archive se décode depuis ses seules séquences : ni en-tête FASTA ni nom de fichier ne
//! sont nécessaires. Un fichier n'est jamais écrit si son empreinte SHA-256, embarquée dans
//! l'archive, ne correspond pas à ce qui a été reconstitué.

mod commands;
mod output;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

use commands::{decode, demux, encode, library, simulate, simulate_library, verify, visualize};

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

    /// Simule la dégradation d'une archive, puis tente de la relire ; ou, avec --library, le
    /// séquençage d'un pool et la restitution de chacun de ses échantillons
    Simulate {
        /// Fichier à archiver ; ou l'archive elle-même avec --archive ; ou le pool avec --library
        #[arg(short, long)]
        input: PathBuf,

        /// L'entrée est une archive FASTA (le fichier d'origine n'est pas nécessaire)
        #[arg(long, conflicts_with = "library")]
        archive: bool,

        /// L'entrée est une bibliothèque de molécules (sortie d'`adn library`) : lectures
        /// appariées, lectures d'index, démultiplexage, décodage de chaque échantillon
        #[arg(long)]
        library: bool,

        #[command(flatten)]
        codec: CodecArgs,

        #[command(flatten)]
        channel: ChannelArgs,

        #[command(flatten)]
        sequencer: SequencerArgs,

        /// Nombre de tirages
        #[arg(short = 'n', long, default_value_t = 10)]
        trials: usize,

        /// Graine du générateur
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },

    /// Assemble des archives en molécules de bibliothèque Illumina, multiplexées (un pool)
    Library {
        /// Un échantillon à assembler, répétable, de la forme ARCHIVE=I7[+I5], ou
        /// NOM=ARCHIVE=I7[+I5] ; un index est un nom Nextera (N501–N508, N701–N712) ou une
        /// séquence ; sans nom, celui du fichier d'archive
        #[arg(long = "sample", value_name = "ARCHIVE=I7[+I5]", required = true)]
        samples: Vec<library::SampleArg>,

        /// Pool à écrire : un enregistrement FASTA par molécule
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Sépare les lectures d'un séquenceur par échantillon et coupe les adaptateurs
    Demux {
        /// Lectures 1 (FASTQ ou FASTA)
        #[arg(long)]
        r1: PathBuf,

        /// Lectures 2
        #[arg(long)]
        r2: PathBuf,

        /// Lectures d'index i7 (I1)
        #[arg(long)]
        i7: PathBuf,

        /// Lectures d'index i5 (I2), pour une bibliothèque à deux index
        #[arg(long)]
        i5: Option<PathBuf>,

        /// Bibliothèque du pool (sortie d'`adn library`) : en donne les échantillons et leurs noms
        #[arg(long)]
        library: Option<PathBuf>,

        /// Un échantillon connu par ses index, répétable, de la forme I7[+I5], ou I7[+I5]=NOM
        #[arg(long = "sample", value_name = "I7[+I5][=NOM]")]
        samples: Vec<demux::DemuxSampleArg>,

        /// Dossier où écrire les lectures de chaque échantillon (NOM.fasta)
        #[arg(short, long)]
        output: PathBuf,

        /// Les en-têtes portent l'échantillon d'origine (lectures d'`adn simulate --library`) :
        /// confronter le rangement à l'origine, et compter les fausses attributions
        #[arg(long)]
        truth: bool,
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
    /// Code ternaire sans base répétée, fenêtres recouvrantes (Goldman et al., 2013)
    #[value(alias = "goldman2013", alias = "goldman_2013")]
    Goldman,
    /// Reed-Solomon sur GF(47), symboles en trimères (Grass et al., 2015)
    #[value(alias = "grass2015", alias = "grass_2015")]
    Grass,
    /// Code convolutif par brin, LDPC par bloc, décodage à décisions souples
    #[value(alias = "viterbi")]
    Ldpc,
    /// Code à hachis par brin, décodé par recherche (Press et al., 2020) : corrige aussi les
    /// insertions et les suppressions
    #[value(alias = "indels")]
    Hedges,
}

impl SchemeArg {
    fn scheme(self) -> adn_core::Scheme {
        match self {
            SchemeArg::Fountain => adn_core::Scheme::Fountain,
            SchemeArg::Strands => adn_core::Scheme::Strands,
            SchemeArg::Goldman => adn_core::Scheme::Goldman,
            SchemeArg::Grass => adn_core::Scheme::Grass,
            SchemeArg::Ldpc => adn_core::Scheme::Ldpc,
            SchemeArg::Hedges => adn_core::Scheme::Hedges,
        }
    }
}

/// Rendement du code de HEDGES : les motifs de bits par base de l'article.
#[derive(Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
enum CodeRateArg {
    /// Une base sur deux porte un bit (300 nt) : ~10 % d'erreurs par base
    #[value(name = "1/4")]
    Quarter,
    /// Deux bits pour trois bases (300 nt) : ~6 %
    #[value(name = "1/3")]
    Third,
    /// Un bit par base (200 nt) : ~3 %
    #[default]
    #[value(name = "1/2")]
    Half,
    /// Trois bits pour deux bases (200 nt) : ~1 %
    #[value(name = "3/4")]
    ThreeQuarters,
}

impl CodeRateArg {
    fn rate(self) -> adn_core::hedges::Rate {
        match self {
            CodeRateArg::Quarter => adn_core::hedges::Rate::QUARTER,
            CodeRateArg::Third => adn_core::hedges::Rate::THIRD,
            CodeRateArg::Half => adn_core::hedges::Rate::HALF,
            CodeRateArg::ThreeQuarters => adn_core::hedges::Rate::THREE_QUARTERS,
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

    /// Parité Reed-Solomon par oligo : octets pour DNA Fountain (2 par défaut, un oligo résiste
    /// à la moitié d'octets erronés et s'allonge de 4 nt par octet), symboles pour grass (14 par
    /// défaut, un nombre pair de 2 à 30 : un brin résiste à la moitié de symboles erronés).
    /// Utile devant des erreurs de synthèse, que toutes les lectures d'un oligo portent à
    /// l'identique. Pour ldpc : brins de parité pour cent brins de données (20 par défaut, de
    /// 5 à 100)
    #[arg(long, value_name = "PARITÉ", value_parser = clap::value_parser!(u8))]
    rs_parity: Option<u8>,

    /// Identifiant du fichier, de 0 à 8 (goldman) : sépare plusieurs archives mélangées dans un
    /// même tube, le décodage n'en retient qu'une
    #[arg(long, value_name = "0-8", value_parser = clap::value_parser!(u8))]
    file_id: Option<u8>,

    /// Rendement du code de HEDGES (hedges) : moins il porte, plus il corrige
    #[arg(long, value_enum, value_name = "1/4|1/3|1/2|3/4")]
    code_rate: Option<CodeRateArg>,

    /// Compression du fichier avant encodage
    #[arg(short = 'c', long, value_enum, default_value_t = CompressionArg::Auto)]
    compression: CompressionArg,

    #[command(flatten)]
    limits: ConstraintArgs,
}

impl CodecArgs {
    fn options(&self) -> anyhow::Result<adn_core::EncodeOptions> {
        use adn_core::codec::{fountain, goldman, grass, ldpc};
        let scheme = self.algorithm.scheme();
        // La parité défaut est celle du schéma : deux octets pour DNA Fountain, quatorze
        // symboles pour grass, vingt pour cent pour ldpc.
        let (rs_parity, accepted) = match scheme {
            adn_core::Scheme::Fountain => (
                self.rs_parity.unwrap_or(fountain::DEFAULT_RS_PARITY as u8),
                1..=32,
            ),
            adn_core::Scheme::Grass => (
                self.rs_parity.unwrap_or(grass::DEFAULT_PARITY as u8),
                grass::MIN_PARITY as u8..=grass::MAX_PARITY as u8,
            ),
            adn_core::Scheme::Ldpc => (
                self.rs_parity.unwrap_or(ldpc::DEFAULT_PARITY as u8),
                ldpc::MIN_PARITY as u8..=ldpc::MAX_PARITY as u8,
            ),
            _ => (0, 0..=0),
        };
        if self.rs_parity.is_some() && !accepted.contains(&rs_parity) {
            anyhow::bail!(
                "--rs-parity doit valoir de {} à {} pour le schéma {}",
                accepted.start(),
                accepted.end(),
                scheme.name()
            );
        }
        let compression = match self.compression {
            CompressionArg::Auto => adn_core::Compression::Auto,
            CompressionArg::None => adn_core::Compression::None,
            CompressionArg::Lz4 => adn_core::Compression::Lz4,
            CompressionArg::Zstd => adn_core::Compression::Zstd,
        };
        if self.file_id.is_some() && scheme != adn_core::Scheme::Goldman {
            anyhow::bail!("--file-id ne s'applique qu'au schéma goldman");
        }
        let default_rate = CodeRateArg::default().rate();
        if self.code_rate.is_some() && scheme != adn_core::Scheme::Hedges {
            anyhow::bail!("--code-rate ne s'applique qu'au schéma hedges");
        }
        let hedges_rate = if scheme == adn_core::Scheme::Hedges {
            self.code_rate.map_or(default_rate, CodeRateArg::rate)
        } else {
            default_rate
        };
        let options = adn_core::EncodeOptions {
            scheme,
            compression,
            fountain: fountain::FountainParams {
                redundancy: self.redundancy,
                rs_parity: usize::from(rs_parity),
                ..adn_core::codec::fountain::FountainParams::default()
            },
            goldman: goldman::GoldmanParams {
                file_id: self.file_id.unwrap_or(0),
            },
            grass: grass::GrassParams {
                parity: usize::from(rs_parity),
            },
            ldpc: ldpc::LdpcParams {
                parity: if scheme == adn_core::Scheme::Ldpc {
                    usize::from(rs_parity)
                } else {
                    ldpc::DEFAULT_PARITY
                },
            },
            hedges: adn_core::codec::hedges::HedgesParams { rate: hedges_rate },
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

    /// Probabilité, par nucléotide lu, qu'une salve d'erreurs commence (0 : aucune)
    #[arg(long, default_value_t = 0.0, value_name = "TAUX")]
    burst_rate: f64,

    /// Longueur moyenne d'une salve, en nucléotides
    #[arg(long, default_value_t = 10.0, value_name = "NUCLÉOTIDES")]
    burst_length: f64,

    /// Taux de substitution dans une salve
    #[arg(long, default_value_t = 0.5, value_name = "TAUX")]
    burst_error: f64,
}

/// Ce que reçoit une simulation : son entrée, ses tirages.
pub(crate) struct RunSpec<'a> {
    pub input: &'a Path,
    pub trials: usize,
    pub seed: u64,
}

/// Paramètres du séquenceur, pour `simulate --library`.
#[derive(clap::Args)]
pub struct SequencerArgs {
    /// Longueur des lectures 1 et 2, en bases
    #[arg(long, default_value_t = 300, value_name = "BASES")]
    pub read_length: usize,

    /// Longueur des lectures d'index ; par défaut celle des index de la bibliothèque
    #[arg(long)]
    pub index_read_length: Option<usize>,

    /// Taux de substitution du séquençage des lectures d'index
    #[arg(long, default_value_t = 0.001, value_name = "TAUX")]
    pub index_substitution_rate: f64,

    /// Fraction des paires de lectures dont les index sont ceux d'une autre molécule du pool
    #[arg(long, default_value_t = 0.0, value_name = "0-1")]
    pub index_hopping: f64,

    /// Écrire les lectures du premier tirage en FASTQ dans ce dossier (R1, R2, I1, I2)
    #[arg(long)]
    pub reads: Option<PathBuf>,
}

impl SequencerArgs {
    /// Sans `--library`, ces paramètres n'ont pas d'effet : refuser ceux qui sont donnés.
    fn check_unused(&self) -> anyhow::Result<()> {
        let defaults = Self {
            read_length: 300,
            index_read_length: None,
            index_substitution_rate: 0.001,
            index_hopping: 0.0,
            reads: None,
        };
        let given = [
            (self.read_length != 300, "--read-length"),
            (self.index_read_length.is_some(), "--index-read-length"),
            (
                self.index_substitution_rate != 0.001,
                "--index-substitution-rate",
            ),
            (self.index_hopping != 0.0, "--index-hopping"),
            (self.reads.is_some(), "--reads"),
        ];
        let given: Vec<&str> = given
            .into_iter()
            .filter(|(set, _)| *set)
            .map(|(_, name)| name)
            .collect();
        if given.is_empty() {
            let _ = defaults;
            Ok(())
        } else {
            anyhow::bail!(
                "{} s'applique{} au séquençage d'une bibliothèque : ajouter --library",
                given.join(", "),
                if given.len() == 1 { "" } else { "nt" }
            )
        }
    }
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
            library,
            codec,
            channel,
            sequencer,
            trials,
            seed,
        } => {
            let spec = RunSpec {
                input: &input,
                trials,
                seed,
            };
            if library {
                simulate_library::run(&spec, &channel, &sequencer, &out)
            } else {
                simulate::run(&spec, archive, &codec, &channel, &sequencer, &out)
            }
        }
        Command::Library { samples, output } => library::run(&samples, &output, &out),
        Command::Demux {
            r1,
            r2,
            i7,
            i5,
            library,
            samples,
            output,
            truth,
        } => demux::run(
            &demux::DemuxArgs {
                r1_path: &r1,
                r2_path: &r2,
                i7_path: &i7,
                i5_path: i5.as_deref(),
                library_path: library.as_deref(),
                samples: &samples,
                output: &output,
                truth,
            },
            &out,
        ),
        Command::Visualize {
            input,
            format,
            output,
            limits,
            all,
        } => visualize::run(&input, format, output, &limits, all, &out),
    }
}
