//! Codecs d'archivage : API commune aux schémas.
//!
//! Un fichier est d'abord enveloppé ([`crate::envelope`]) : compression, longueurs et empreinte
//! SHA-256. Le corps obtenu est réparti dans des oligos selon un schéma, et des oligos
//! « manifestes » ([`manifest`]) portent les paramètres nécessaires à la lecture. Le décodage ne
//! dépend d'aucune information extérieure aux séquences : ni en-tête FASTA, ni nom de fichier.
//!
//! Aucun fichier n'est rendu si son empreinte SHA-256 n'est pas vérifiée.

pub mod fountain;
pub mod goldman;
pub mod grass;
pub mod lt;
pub mod manifest;
pub mod rng;
pub mod soliton;
pub mod strands;

use crate::checksum::sha256;
use crate::constraints::DnaConstraints;
use crate::envelope::{self, Compression};
use crate::error::{DnaError, Result};

/// Noms des schémas retirés, qui ne mettaient pas en œuvre ce que leur nom annonçait.
pub const RETIRED_SCHEMES: [&str; 3] = ["adaptive", "base3", "ultimate"];

/// Nombre minimal d'oligos manifestes dans une archive.
const MANIFEST_COPIES_MIN: usize = 12;
/// Nombre maximal d'oligos manifestes dans une archive.
const MANIFEST_COPIES_MAX: usize = 64;
/// Parité des oligos manifestes d'une archive goldman (2 octets : 152 nt).
const GOLDMAN_MANIFEST_PARITY: u8 = 2;

/// Noms de tous les schémas, pour les messages d'erreur.
const SCHEME_NAMES: &str = "fountain, strands, goldman, grass";

/// Schéma de codage du corps d'une archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    /// DNA Fountain (Erlich et Zielinski, 2017).
    Fountain,
    /// Brins codés, à deux niveaux de Reed-Solomon.
    Strands,
    /// Code de Huffman ternaire, sans base répétée, fenêtres recouvrantes (Goldman et al., 2013).
    Goldman,
    /// Reed-Solomon sur GF(47), un symbole par trimère (Grass et al., 2015).
    Grass,
}

impl Scheme {
    /// Tous les schémas.
    pub const ALL: [Scheme; 4] = [
        Scheme::Fountain,
        Scheme::Strands,
        Scheme::Goldman,
        Scheme::Grass,
    ];

    /// Nom utilisé en ligne de commande et dans l'API.
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Fountain => "fountain",
            Scheme::Strands => "strands",
            Scheme::Goldman => "goldman",
            Scheme::Grass => "grass",
        }
    }

    /// Titre affichable.
    pub fn label(self) -> &'static str {
        match self {
            Scheme::Fountain => "DNA Fountain (Erlich et Zielinski, 2017)",
            Scheme::Strands => "Brins codés (Reed-Solomon interne et externe)",
            Scheme::Goldman => "Goldman et al., 2013 (code ternaire sans base répétée)",
            Scheme::Grass => "Grass et al., 2015 (Reed-Solomon sur GF(47), symboles en trimères)",
        }
    }

    /// Vrai si le schéma garantit, brin par brin, les bornes de GC des contraintes.
    ///
    /// Le schéma goldman ne le peut pas, pas plus que l'article : sa table de conversion est une
    /// bijection, sans marge de manœuvre.
    pub fn constrains_gc(self) -> bool {
        self != Scheme::Goldman
    }

    /// Retrouve un schéma par son nom. Les alias historiques sont acceptés.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "fountain" | "ez2017" | "erlich_zielinski_2017" | "dna_fountain" => {
                Some(Scheme::Fountain)
            }
            "strands" | "reed_solomon" | "rs" => Some(Scheme::Strands),
            "goldman" | "goldman2013" | "goldman_2013" => Some(Scheme::Goldman),
            "grass" | "grass2015" | "grass_2015" => Some(Scheme::Grass),
            _ => None,
        }
    }
}

/// Retrouve un schéma par son nom, avec une erreur explicite sinon.
pub fn parse_scheme(name: &str) -> Result<Scheme> {
    let key = name.trim().to_ascii_lowercase();
    if RETIRED_SCHEMES.contains(&key.as_str()) {
        return Err(DnaError::UnsupportedFormat(format!(
            "le schéma « {} » a été retiré : il ne mettait pas en œuvre ce que son nom \
             annonçait. Schémas disponibles : {SCHEME_NAMES}.",
            name.trim()
        )));
    }
    Scheme::from_name(name).ok_or_else(|| {
        DnaError::InvalidConfig(format!(
            "schéma « {} » inconnu. Schémas disponibles : {SCHEME_NAMES}.",
            name.trim()
        ))
    })
}

/// Longueur des oligos d'un schéma avec les options par défaut, en nucléotides.
pub fn oligo_length(scheme: Scheme) -> usize {
    EncodeOptions::for_scheme(scheme).oligo_length()
}

/// Options d'encodage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncodeOptions {
    pub scheme: Scheme,
    pub compression: Compression,
    /// Paramètres du schéma DNA Fountain. La redondance ne s'applique qu'à ce schéma.
    pub fountain: fountain::FountainParams,
    /// Paramètres du schéma goldman.
    pub goldman: goldman::GoldmanParams,
    /// Paramètres du schéma grass.
    pub grass: grass::GrassParams,
    /// Contraintes biochimiques que doit respecter chaque oligo.
    pub constraints: DnaConstraints,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            scheme: Scheme::Fountain,
            compression: Compression::Auto,
            fountain: fountain::FountainParams::default(),
            goldman: goldman::GoldmanParams::default(),
            grass: grass::GrassParams::default(),
            constraints: DnaConstraints::default(),
        }
    }
}

impl EncodeOptions {
    /// Options pour un schéma donné, le reste par défaut.
    pub fn for_scheme(scheme: Scheme) -> Self {
        Self {
            scheme,
            ..Self::default()
        }
    }

    /// Longueur des oligos de données que ces options produisent, en nucléotides. Elle dépend de
    /// la parité des oligos DNA Fountain : 4 nucléotides de plus par octet.
    pub fn oligo_length(&self) -> usize {
        match self.scheme {
            Scheme::Fountain => self.fountain.oligo_len(),
            Scheme::Strands => strands::STRAND_LEN,
            Scheme::Goldman => goldman::STRAND_LEN,
            Scheme::Grass => grass::STRAND_LEN,
        }
    }

    /// Vérifie la cohérence des options.
    pub fn validate(&self) -> Result<()> {
        self.constraints.validate()?;
        match self.scheme {
            Scheme::Fountain => self.fountain.validate(),
            Scheme::Strands => Ok(()),
            Scheme::Goldman => {
                self.goldman.validate()?;
                if !self.constraints.gc_is_default() && !self.constraints.gc_is_free() {
                    return Err(DnaError::InvalidConfig(
                        "le schéma goldman ne règle pas la teneur en GC de ses oligos (50 % en \
                         moyenne, comme dans l'article) : laisser les bornes de GC par défaut, \
                         ou les ouvrir de 0 à 1"
                            .to_string(),
                    ));
                }
                Ok(())
            }
            Scheme::Grass => {
                self.grass.validate()?;
                if self.constraints.max_homopolymer < 3 {
                    return Err(DnaError::InvalidConfig(
                        "le schéma grass garantit des suites d'au plus 3 bases (les trimères de \
                         l'article) : --max-homopolymer doit valoir 3 ou plus"
                            .to_string(),
                    ));
                }
                Ok(())
            }
        }
    }
}

/// Bilan d'un encodage.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EncodeStats {
    pub scheme: Scheme,
    /// Longueur du fichier original.
    pub original_len: u64,
    /// Longueur de l'enveloppe, répartie dans les oligos.
    pub body_len: u64,
    pub compression: Compression,
    /// Blocs (DNA Fountain) ou brins de données (brins codés).
    pub units: u32,
    pub data_oligos: usize,
    pub manifest_copies: usize,
    /// Total de nucléotides, manifestes compris.
    pub nucleotides: u64,
    /// Bits du fichier original par nucléotide.
    pub bits_per_nucleotide: f64,
    /// Candidats essayés par le criblage : graines (DNA Fountain) ou nonces (brins codés).
    pub candidates_tried: u64,
    pub sha256: [u8; 32],
}

/// Archive encodée.
#[derive(Clone, Debug)]
pub struct Encoded {
    /// Oligos, manifestes en tête.
    pub oligos: Vec<Vec<u8>>,
    pub stats: EncodeStats,
}

/// Bilan d'un décodage.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct DecodeStats {
    /// Lectures reçues.
    pub reads_total: usize,
    /// Lectures vides ou contenant autre chose que A, C, G, T.
    pub reads_invalid: usize,
    /// Lectures de données dont la longueur n'est pas celle d'un oligo du schéma.
    pub reads_bad_length: usize,
    /// Lectures dont le début est celui d'un oligo manifeste.
    pub manifest_candidates: usize,
    /// Lectures dont le manifeste a été lu et confirme le manifeste retenu.
    pub manifest_agreeing: usize,
    /// Lectures corrigées par le code interne.
    pub corrected: usize,
    /// Lectures irrécupérables par le code interne.
    pub uncorrectable: usize,
    /// Gouttes ou brins distincts utilisés.
    pub units_used: usize,
    /// Index écartés : lectures en désaccord sur un même oligo.
    pub contested: usize,
    /// Blocs réparés par le code externe (brins codés).
    pub blocks_repaired: usize,
    /// Brins restitués par le code externe (brins codés).
    pub strands_repaired: usize,
    /// Le premier passage a échoué au contrôle d'intégrité, et le second, sans les lectures
    /// corrigées par la parité, a réussi (DNA Fountain).
    pub retried_without_corrected_reads: bool,
}

/// Fichier reconstitué et vérifié.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub data: Vec<u8>,
    pub scheme: Scheme,
    pub compression: Compression,
    pub sha256: [u8; 32],
    pub stats: DecodeStats,
}

/// Options de décodage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Force un schéma ; par défaut, c'est le manifeste de l'archive qui le détermine.
    pub scheme: Option<Scheme>,
}

/// Nombre d'oligos manifestes : un dixième des oligos de données, entre 12 et 64.
fn manifest_copies(data_oligos: usize) -> usize {
    (data_oligos / 10).clamp(MANIFEST_COPIES_MIN, MANIFEST_COPIES_MAX)
}

fn u64_len(len: usize) -> Result<u64> {
    u64::try_from(len).map_err(|_| DnaError::Encoding("fichier trop volumineux".to_string()))
}

/// Encode un fichier en oligos.
pub fn encode(data: &[u8], options: &EncodeOptions) -> Result<Encoded> {
    options.validate()?;
    let body = envelope::pack(data, options.compression)?;
    let compression = envelope::compression_of(&body)?;
    let body_len = u64_len(body.len())?;

    let (manifest, data_oligos, units, candidates_tried) = match options.scheme {
        Scheme::Fountain => {
            let params = options.fountain;
            let rs_parity = u8::try_from(params.rs_parity).map_err(|_| {
                DnaError::InvalidConfig(
                    "parité Reed-Solomon trop grande (255 octets maximum)".to_string(),
                )
            })?;
            let chunks = fountain::chunks_for(body.len()).ok_or_else(|| {
                DnaError::Encoding(
                    "fichier trop volumineux pour le format (137 Gio au maximum)".to_string(),
                )
            })?;
            let manifest = manifest::Manifest {
                scheme: manifest::SchemeId::Fountain,
                rs_parity,
                compression,
                body_len,
                chunks,
                c: params.effective_c(chunks) as f32,
                delta: params.delta as f32,
                file_id: 0,
            };
            let (oligos, stats) =
                fountain::encode(&body, &manifest, &params, &options.constraints)?;
            (manifest, oligos, stats.chunks, stats.seeds_tried)
        }
        Scheme::Strands => {
            let manifest = manifest::Manifest {
                scheme: manifest::SchemeId::Strands,
                rs_parity: strands::INNER_PARITY as u8,
                compression,
                body_len,
                chunks: 0,
                c: 0.0,
                delta: 0.0,
                file_id: 0,
            };
            let (oligos, stats) = strands::encode(&body, &options.constraints)?;
            (manifest, oligos, stats.data_strands, stats.nonces_tried)
        }
        Scheme::Goldman => {
            let (oligos, stats) = goldman::encode(&body, &options.goldman)?;
            let manifest = manifest::Manifest {
                scheme: manifest::SchemeId::Goldman,
                rs_parity: GOLDMAN_MANIFEST_PARITY,
                compression,
                body_len,
                chunks: stats.segments,
                c: 0.0,
                delta: 0.0,
                file_id: options.goldman.file_id,
            };
            // Pas de criblage : la conversion des trits en bases est une bijection, sans
            // candidat de rechange.
            (manifest, oligos, stats.segments, 0)
        }
        Scheme::Grass => {
            let parity = u8::try_from(options.grass.parity)
                .map_err(|_| DnaError::InvalidConfig("parité interne trop grande".to_string()))?;
            let (oligos, stats) = grass::encode(&body, &options.grass, &options.constraints)?;
            // La parité du manifeste est celle des brins : l'oligo manifeste fait 4 nt de plus
            // par symbole de parité.
            let manifest = manifest::Manifest {
                scheme: manifest::SchemeId::Grass,
                rs_parity: parity,
                compression,
                body_len,
                chunks: 0,
                c: 0.0,
                delta: 0.0,
                file_id: 0,
            };
            (
                manifest,
                oligos,
                stats.data_strands as u32,
                stats.nonces_tried,
            )
        }
    };

    let data_oligo_count = data_oligos.len();
    let copies = manifest_copies(data_oligo_count);
    let rs = crate::rs::RsCode::new(usize::from(manifest.rs_parity))?;
    let mut oligos = Vec::with_capacity(data_oligos.len() + copies);
    for copy in 0..copies {
        oligos.push(manifest::oligo(
            &manifest,
            &rs,
            &options.constraints,
            copy as u32 * 997,
        )?);
    }
    oligos.extend(data_oligos);

    let nucleotides: u64 = oligos.iter().map(|oligo| oligo.len() as u64).sum();
    let bits_per_nucleotide = if nucleotides == 0 {
        0.0
    } else {
        u64_len(data.len())? as f64 * 8.0 / nucleotides as f64
    };
    Ok(Encoded {
        oligos,
        stats: EncodeStats {
            scheme: options.scheme,
            original_len: u64_len(data.len())?,
            body_len,
            compression,
            units,
            data_oligos: data_oligo_count,
            manifest_copies: copies,
            nucleotides,
            bits_per_nucleotide,
            candidates_tried,
            sha256: sha256(data),
        },
    })
}

/// Décode le corps d'une archive DNA Fountain.
///
/// Un premier passage utilise toutes les lectures valides. Si le corps ne passe pas le contrôle
/// d'intégrité de l'enveloppe, une lecture « corrigée » par la parité l'a probablement été à
/// tort : on réessaie sans aucune de ces lectures. L'empreinte finale reste l'arbitre.
fn decode_fountain_body(
    reads: &[Vec<u8>],
    manifest: &manifest::Manifest,
) -> Result<(Vec<u8>, fountain::DecodeStats, bool)> {
    let (body, first) = fountain::decode(reads, manifest, false)?;
    if envelope::inspect(&body).is_ok() {
        return Ok((body, first, false));
    }

    let second = fountain::decode(reads, manifest, true);
    let Ok((body, mut second)) = second else {
        return Err(DnaError::Integrity(
            "le corps reconstitué est incohérent, et les lectures reçues sans erreur ne \
             suffisent pas à le reconstruire : erreurs trop nombreuses, ou parité trop faible"
                .to_string(),
        ));
    };
    envelope::inspect(&body).map_err(|_| {
        DnaError::Integrity(
            "le corps reconstitué est incohérent, même sans les lectures corrigées : erreurs \
             trop nombreuses, ou archive altérée"
                .to_string(),
        )
    })?;
    // Le bilan du premier passage décrit toutes les lectures reçues.
    second.corrected = first.corrected;
    second.uncorrectable = first.uncorrectable;
    Ok((body, second, true))
}

/// Décode des lectures (séquences seules) en fichier vérifié.
pub fn decode<I, S>(reads: I, options: &DecodeOptions) -> Result<Decoded>
where
    I: IntoIterator<Item = S>,
    S: AsRef<[u8]>,
{
    // 1. Normalisation : majuscules et blancs tolérés ; tout autre caractère écarte la lecture.
    let mut normalized: Vec<Vec<u8>> = Vec::new();
    let mut stats = DecodeStats::default();
    for read in reads {
        stats.reads_total += 1;
        match crate::sequence::normalize(read.as_ref()) {
            Ok(sequence) if !sequence.is_empty() => normalized.push(sequence),
            _ => stats.reads_invalid += 1,
        }
    }

    // 2. Manifeste : il donne le schéma et les paramètres de l'archive.
    let (manifest, data_reads, manifest_stats) = manifest::find(normalized)?;
    stats.manifest_candidates = manifest_stats.candidates;
    stats.manifest_agreeing = manifest_stats.agreeing;
    let scheme = match manifest.scheme {
        manifest::SchemeId::Fountain => Scheme::Fountain,
        manifest::SchemeId::Strands => Scheme::Strands,
        manifest::SchemeId::Goldman => Scheme::Goldman,
        manifest::SchemeId::Grass => Scheme::Grass,
    };
    if options.scheme.is_some_and(|forced| forced != scheme) {
        return Err(DnaError::UnsupportedFormat(format!(
            "l'archive est au schéma « {} », pas « {} »",
            scheme.name(),
            options.scheme.map_or("-", Scheme::name)
        )));
    }

    // 3. Corps, selon le schéma.
    let body_len = usize::try_from(manifest.body_len)
        .map_err(|_| DnaError::UnsupportedFormat("longueur de corps aberrante".to_string()))?;
    let (body, reads_used) = match scheme {
        Scheme::Fountain => {
            let (body, inner, retried) = decode_fountain_body(&data_reads, &manifest)?;
            stats.corrected = inner.corrected;
            stats.uncorrectable = inner.uncorrectable;
            stats.units_used = inner.droplets;
            stats.contested = inner.contested;
            stats.retried_without_corrected_reads = retried;
            (body, inner.reads_used)
        }
        Scheme::Strands => {
            let (body, inner) = strands::decode(&data_reads, body_len)?;
            stats.corrected = inner.corrected;
            stats.uncorrectable = inner.uncorrectable;
            stats.units_used = inner.strands;
            stats.contested = inner.contested;
            stats.blocks_repaired = inner.blocks_repaired;
            stats.strands_repaired = inner.strands_repaired;
            (body, inner.reads_used)
        }
        Scheme::Goldman => {
            let layout = goldman::SegmentLayout::from_manifest(manifest.chunks, manifest.body_len)?;
            let (body, inner) = goldman::decode(&data_reads, &layout, manifest.file_id)?;
            stats.corrected = inner.corrected;
            stats.uncorrectable = inner.uncorrectable;
            stats.units_used = inner.segments;
            stats.contested = inner.contested;
            (body, inner.reads_used)
        }
        Scheme::Grass => {
            let (body, inner) =
                grass::decode(&data_reads, body_len, usize::from(manifest.rs_parity))?;
            stats.corrected = inner.corrected;
            stats.uncorrectable = inner.uncorrectable;
            stats.units_used = inner.strands;
            stats.contested = inner.contested;
            stats.blocks_repaired = inner.blocks_repaired;
            stats.strands_repaired = inner.strands_repaired;
            (body, inner.reads_used)
        }
    };
    stats.reads_bad_length = data_reads.len().saturating_sub(reads_used);

    // 4. Enveloppe : CRC, décompression, empreinte SHA-256.
    let data = envelope::unpack(&body)?;
    let sha256 = sha256(&data);
    Ok(Decoded {
        data,
        scheme,
        compression: manifest.compression,
        sha256,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::hex;
    use crate::codec::fountain::FountainParams;
    use crate::rs::RsCode;
    use crate::sequence::{bytes_to_dna, dna_to_bytes};

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut rng = rng::SeededRng::new(seed, 3);
        let mut out = vec![0u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    #[test]
    fn decodes_from_bare_sequences_in_any_order_for_every_scheme() {
        for scheme in Scheme::ALL {
            let data = b"donnees a archiver, sans aucune information exterieure. ".repeat(400);
            let options = EncodeOptions::for_scheme(scheme);
            let encoded = encode(&data, &options).unwrap();
            assert_eq!(
                encoded.stats.data_oligos + encoded.stats.manifest_copies,
                encoded.oligos.len()
            );
            let constraints = DnaConstraints::default();
            let copies = encoded.stats.manifest_copies;
            // La parité d'un oligo manifeste suit celle du schéma.
            let manifest_parity = match scheme {
                Scheme::Fountain => options.fountain.rs_parity as u8,
                Scheme::Strands => strands::INNER_PARITY as u8,
                Scheme::Goldman => GOLDMAN_MANIFEST_PARITY,
                Scheme::Grass => options.grass.parity as u8,
            };
            for (rank, oligo) in encoded.oligos.iter().enumerate() {
                // Les manifestes sont en 2 bits par nucléotide ; les brins des deux derniers
                // schémas portent autre chose (un trit ou un symbole de GF(47) par groupe).
                let expected = if rank < copies {
                    manifest::oligo_length(manifest_parity)
                } else {
                    oligo_length(scheme)
                };
                assert_eq!(oligo.len(), expected);
                // Le schéma goldman ne garantit pas le GC de ses brins (50 % en moyenne).
                if rank < copies || scheme.constrains_gc() {
                    constraints.check(oligo).unwrap();
                }
            }

            // Lectures nues, en ordre inverse, en minuscules, avec des blancs : uniquement des
            // séquences, aucun en-tête.
            let mut reads: Vec<Vec<u8>> = encoded.oligos.clone();
            reads.reverse();
            let reads: Vec<String> = reads
                .iter()
                .map(|oligo| {
                    String::from_utf8(oligo.to_vec())
                        .unwrap()
                        .to_lowercase()
                        .chars()
                        .collect::<Vec<_>>()
                        .chunks(60)
                        .map(|chunk| chunk.iter().collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .collect();
            let decoded = decode(&reads, &DecodeOptions::default()).unwrap();
            assert_eq!(decoded.data, data, "{scheme:?}");
            assert_eq!(decoded.scheme, scheme);
            assert_eq!(hex(&decoded.sha256), hex(&sha256(&data)));
            assert_eq!(decoded.stats.reads_invalid, 0);
            assert_eq!(decoded.stats.reads_bad_length, 0);
        }
    }

    #[test]
    fn survives_down_to_one_manifest_copy() {
        let data = pseudo_random(10_000, 1);
        let encoded = encode(&data, &EncodeOptions::default()).unwrap();
        let copies = encoded.stats.manifest_copies;
        assert!(copies >= MANIFEST_COPIES_MIN);

        let reads: Vec<Vec<u8>> = encoded.oligos[copies - 1..].to_vec();
        assert_eq!(
            decode(&reads, &DecodeOptions::default()).unwrap().data,
            data
        );

        // Sans aucun manifeste : échec annoncé.
        let reads: Vec<Vec<u8>> = encoded.oligos[copies..].to_vec();
        let err = decode(&reads, &DecodeOptions::default()).unwrap_err();
        assert!(matches!(err, DnaError::UnsupportedFormat(_)), "{err}");
    }

    #[test]
    fn empty_file_and_invalid_reads_are_handled() {
        let encoded = encode(b"", &EncodeOptions::default()).unwrap();
        let decoded = decode(&encoded.oligos, &DecodeOptions::default()).unwrap();
        assert!(decoded.data.is_empty());

        let mut reads = encoded.oligos.clone();
        reads.push(b"ACGN".to_vec());
        reads.push(Vec::new());
        reads.push(b"acgt".to_vec());
        let decoded = decode(&reads, &DecodeOptions::default()).unwrap();
        assert!(decoded.data.is_empty());
        // Une lecture avec un N et une lecture vide.
        assert_eq!(decoded.stats.reads_invalid, 2);
        assert_eq!(decoded.stats.reads_bad_length, 1);

        assert!(decode(Vec::<Vec<u8>>::new(), &DecodeOptions::default()).is_err());
    }

    #[test]
    fn forced_scheme_must_match_the_archive() {
        let encoded = encode(b"abc", &EncodeOptions::default()).unwrap();
        let options = DecodeOptions {
            scheme: Some(Scheme::Strands),
        };
        assert!(matches!(
            decode(&encoded.oligos, &options),
            Err(DnaError::UnsupportedFormat(_))
        ));
        let options = DecodeOptions {
            scheme: Some(Scheme::Fountain),
        };
        assert!(decode(&encoded.oligos, &options).is_ok());
    }
    /// Une altération de deux octets de la charge utile que la parité « corrige » à tort.
    /// La lecture est renvoyée telle qu'elle arrive : parité invalide.
    fn false_correction(oligo: &[u8], rs: &RsCode) -> Option<Vec<u8>> {
        let block = dna_to_bytes(oligo).ok()?;
        for first in (fountain::SEED_LEN..block.len()).step_by(7) {
            for second in (first + 1..block.len()).step_by(5) {
                let mut damaged = block.clone();
                damaged[first] ^= 0x01;
                damaged[second] ^= 0x80;
                let mut corrected = damaged.clone();
                if rs.correct(&mut corrected).is_ok() && corrected != block {
                    let mut dna = Vec::new();
                    bytes_to_dna(&damaged, &mut dna);
                    return Some(dna);
                }
            }
        }
        None
    }

    #[test]
    fn a_falsely_corrected_read_is_set_aside_by_the_second_pass() {
        use crate::codec::lt::{LayoutSampler, DEFAULT_DELTA};
        use crate::codec::manifest::SchemeId;

        // Un corps de trois blocs (en-tête de 60 octets, 36 octets de données).
        let data = pseudo_random(36, 1);
        let body = envelope::pack(&data, Compression::None).unwrap();
        let chunks = fountain::chunks_for(body.len()).unwrap();
        assert_eq!(chunks, 3);
        let params = FountainParams::default();
        let manifest = manifest::Manifest {
            scheme: SchemeId::Fountain,
            rs_parity: fountain::DEFAULT_RS_PARITY as u8,
            compression: Compression::None,
            body_len: body.len() as u64,
            chunks,
            c: params.effective_c(chunks) as f32,
            delta: DEFAULT_DELTA as f32,
            file_id: 0,
        };
        let rs = RsCode::new(fountain::DEFAULT_RS_PARITY).unwrap();

        // Quatre gouttes suffisent : {0}, {1,2}, {2}, {1}. Les gouttes sont imposées par la
        // graine ; on cherche des graines croissantes dans cet ordre, de sorte que la goutte
        // {1,2} soit déjà en attente quand {2} résout le bloc 2, et avant que {1} n'arrive.
        let mut sampler =
            LayoutSampler::new(chunks, f64::from(manifest.c), f64::from(manifest.delta)).unwrap();
        let mut indices = Vec::new();
        let mut wanted: Vec<Vec<u32>> = vec![vec![1, 2], vec![2], vec![1]];
        let mut seeds: Vec<u32> = Vec::new();
        for seed in 0..100_000u32 {
            sampler.layout(seed, &mut indices);
            if indices == wanted[0] {
                seeds.push(seed);
                wanted.remove(0);
                if wanted.is_empty() {
                    break;
                }
            }
        }
        assert_eq!(wanted.len(), 0, "graines trouvées: {seeds:?}");
        // Et une goutte {0}, de préférence hors de cet intervalle.
        let zero = (0..100_000u32)
            .find(|&seed| {
                let mut probe =
                    LayoutSampler::new(chunks, f64::from(manifest.c), DEFAULT_DELTA).unwrap();
                probe.layout(seed, &mut indices);
                !seeds.contains(&seed) && indices == [0]
            })
            .expect("une graine {0}");
        let chunk =
            |index: usize| body[index * fountain::CHUNK_LEN..][..fountain::CHUNK_LEN].to_vec();
        let mut mixed = chunk(1);
        for (slot, &byte) in mixed.iter_mut().zip(chunk(2).iter()) {
            *slot ^= byte;
        }
        let oligo =
            |seed: u32, payload: &[u8]| fountain::oligo_for_droplet(seed, payload, &rs).unwrap();
        let mixed = oligo(seeds[0], &mixed);
        let last = oligo(seeds[1], &chunk(2));
        let middle = oligo(seeds[2], &chunk(1));
        let first = oligo(zero, &chunk(0));
        let poisoned = false_correction(&mixed, &rs).expect("une fausse correction doit exister");

        // Sans altération : un seul passage.
        let reads = vec![first.clone(), mixed.clone(), last.clone(), middle.clone()];
        let (recovered, _, retried) = decode_fountain_body(&reads, &manifest).unwrap();
        assert_eq!(recovered, body);
        assert!(!retried);

        // La goutte {1,2} falsifiée empoisonne le premier passage : elle résout le bloc 1 à
        // tort avant l'arrivée de {1}. Le second l'écarte, {1} prend le relais.
        let reads = vec![first, poisoned.clone(), last, middle];
        let (recovered, _, retried) = decode_fountain_body(&reads, &manifest).unwrap();
        assert_eq!(recovered, body);
        assert!(retried);

        // Sans la goutte {1} de secours : le second passage ne peut rien, l'échec est annoncé.
        let reads = vec![oligo(zero, &chunk(0)), poisoned, oligo(seeds[1], &chunk(2))];
        let err = decode_fountain_body(&reads, &manifest).unwrap_err();
        assert!(matches!(err, DnaError::Integrity(_)), "{err}");
    }

    #[test]
    fn scheme_names_and_retired_schemes() {
        for scheme in Scheme::ALL {
            assert_eq!(Scheme::from_name(scheme.name()), Some(scheme));
            assert_eq!(parse_scheme(scheme.name()).unwrap(), scheme);
        }
        assert_eq!(Scheme::from_name("EZ2017"), Some(Scheme::Fountain));
        assert_eq!(Scheme::from_name("rs"), Some(Scheme::Strands));
        assert_eq!(Scheme::from_name("Goldman2013"), Some(Scheme::Goldman));
        assert_eq!(parse_scheme("goldman_2013").unwrap(), Scheme::Goldman);
        assert_eq!(Scheme::from_name("Grass2015"), Some(Scheme::Grass));
        assert!(Scheme::from_name("ultimate").is_none());
        for retired in RETIRED_SCHEMES {
            let err = parse_scheme(retired).unwrap_err();
            assert!(matches!(err, DnaError::UnsupportedFormat(_)));
            assert!(err.to_string().contains(SCHEME_NAMES), "{err}");
        }
        let err = parse_scheme("choucroute").unwrap_err();
        assert!(matches!(err, DnaError::InvalidConfig(_)));
        assert!(err.to_string().contains(SCHEME_NAMES), "{err}");
        for scheme in Scheme::ALL {
            assert!(scheme.constrains_gc() == (scheme != Scheme::Goldman));
        }
    }

    #[test]
    fn manifest_copies_scale_with_the_archive() {
        assert_eq!(manifest_copies(0), MANIFEST_COPIES_MIN);
        assert_eq!(manifest_copies(2_000), MANIFEST_COPIES_MAX);
        assert_eq!(manifest_copies(200), 20);
    }

    #[test]
    fn options_are_validated() {
        let mut options = EncodeOptions::default();
        options.fountain.redundancy = 0.2;
        assert!(encode(b"x", &options).is_err());
        options = EncodeOptions::default();
        options.constraints.max_homopolymer = 0;
        assert!(encode(b"x", &options).is_err());
        options = EncodeOptions::for_scheme(Scheme::Strands);
        options.fountain.redundancy = 0.2; // sans effet pour ce schéma
        assert!(encode(b"x", &options).is_ok());
        // Le schéma goldman n'a pas de prise sur le GC : des bornes particulières sont refusées,
        // les bornes par défaut ou ouvertes sont admises.
        options = EncodeOptions::for_scheme(Scheme::Goldman);
        assert!(encode(b"x", &options).is_ok());
        options.constraints = DnaConstraints::new(0.40, 0.60, 3);
        assert!(matches!(
            encode(b"x", &options),
            Err(DnaError::InvalidConfig(_))
        ));
        options.constraints = DnaConstraints::new(0.0, 1.0, 3);
        assert!(encode(b"x", &options).is_ok());
        options.goldman.file_id = 9;
        assert!(matches!(
            encode(b"x", &options),
            Err(DnaError::InvalidConfig(_))
        ));
        // Le schéma grass garantit des suites d'au plus 3 bases : pas moins.
        options = EncodeOptions::for_scheme(Scheme::Grass);
        assert!(encode(b"x", &options).is_ok());
        options.constraints.max_homopolymer = 2;
        assert!(matches!(
            encode(b"x", &options),
            Err(DnaError::InvalidConfig(_))
        ));
        options.constraints.max_homopolymer = 3;
        options.grass.parity = 1;
        assert!(matches!(
            encode(b"x", &options),
            Err(DnaError::InvalidConfig(_))
        ));
        options.grass.parity = 30;
        assert!(encode(b"x", &options).is_ok());
    }

    /// Abîme une archive goldman par des substitutions tirées au hasard dans chaque lecture.
    fn damaged_goldman_reads(archive: &Encoded, rate: f64, seed: u64) -> Vec<Vec<u8>> {
        let mut rng = rng::SeededRng::new(seed, 19);
        archive
            .oligos
            .iter()
            .map(|oligo| {
                oligo
                    .iter()
                    .map(|&base| {
                        if rng.next_f64() < rate {
                            crate::sequence::BASES[rng.below(4) as usize]
                        } else {
                            base
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_damaged_goldman_archive_never_yields_a_false_file() {
        let data = pseudo_random(3_000, 23);
        let options = EncodeOptions::for_scheme(Scheme::Goldman);
        let archive = encode(&data, &options).unwrap();
        assert_eq!(archive.stats.units, archive.stats.data_oligos as u32);
        // À couverture 1, chaque base du message a quatre lectures : le vote traverse 0,1 % de
        // substitutions par base. Au-delà, les lectures dont l'index est atteint sont perdues,
        // et une égalité peut rester : l'échec est annoncé, jamais un fichier faux.
        let recovered = (0..20u64)
            .filter(|&seed| {
                decode(
                    damaged_goldman_reads(&archive, 0.001, seed),
                    &DecodeOptions::default(),
                )
                .is_ok()
            })
            .count();
        assert!(recovered >= 18, "{recovered} récupérations sur 20");

        for rate in [0.002, 0.005, 0.01, 0.03, 0.08] {
            let mut failures = Vec::new();
            for seed in 0..20u64 {
                let reads = damaged_goldman_reads(&archive, rate, seed);
                match decode(&reads, &DecodeOptions::default()) {
                    // L'empreinte SHA-256 de l'enveloppe ne laisse rien passer d'autre.
                    Ok(decoded) => assert_eq!(decoded.data, data, "taux {rate}, graine {seed}"),
                    Err(DnaError::InsufficientData(_)) | Err(DnaError::Integrity(_)) => {
                        failures.push(seed)
                    }
                    // À ce taux, les manifestes eux-mêmes deviennent illisibles.
                    Err(error @ DnaError::UnsupportedFormat(_)) if rate >= 0.08 => {
                        assert!(error.to_string().contains("manifeste"), "{error}");
                        failures.push(seed);
                    }
                    Err(error) => panic!("taux {rate}, graine {seed} : {error}"),
                }
            }
            if rate >= 0.01 {
                assert!(!failures.is_empty(), "taux {rate} : aucun échec annoncé");
            }
        }
    }
}
