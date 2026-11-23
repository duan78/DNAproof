//! Inspection d'un ensemble d'oligos : longueurs, contenu en GC, homopolymères et respect des
//! contraintes biochimiques.

use crate::constraints::DnaConstraints;
use crate::sequence::{gc_content, max_homopolymer};
use serde::Serialize;

/// Statistiques d'un oligo.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OligoReport {
    /// Rang de la séquence dans l'ensemble.
    pub index: usize,
    pub length: usize,
    /// Part de G et C.
    pub gc: f64,
    pub max_homopolymer: usize,
    pub compliant: bool,
}

/// Statistiques d'un ensemble d'oligos.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PoolReport {
    pub oligos: usize,
    pub nucleotides: u64,
    /// Plus petite longueur ; 0 si l'ensemble est vide.
    pub length_min: usize,
    pub length_max: usize,
    pub gc_min: f64,
    pub gc_mean: f64,
    pub gc_max: f64,
    pub max_homopolymer: usize,
    pub compliant: usize,
    pub constraints: DnaConstraints,
}

/// Statistiques de chaque oligo, et bilan de l'ensemble.
pub fn inspect(
    records: &[crate::fasta::Record],
    constraints: &DnaConstraints,
) -> (PoolReport, Vec<OligoReport>) {
    let per_oligo: Vec<OligoReport> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let sequence = &record.sequence;
            OligoReport {
                index,
                length: sequence.len(),
                gc: gc_content(sequence),
                max_homopolymer: max_homopolymer(sequence),
                compliant: constraints.satisfied(sequence),
            }
        })
        .collect();
    (aggregate(&per_oligo, *constraints), per_oligo)
}

/// Bilan d'un ensemble d'oligos.
pub fn pool_report(records: &[crate::fasta::Record], constraints: &DnaConstraints) -> PoolReport {
    inspect(records, constraints).0
}

/// Bilan d'un ensemble d'oligos d'un schéma donné.
///
/// Le schéma goldman ne garantit pas le GC de ses brins : la contrainte de GC n'y compte pas,
/// seule celle des suites identiques. Les manifestes, eux, la respectent comme partout.
pub fn pool_report_of_scheme(
    records: &[crate::fasta::Record],
    constraints: &DnaConstraints,
    scheme: Option<crate::Scheme>,
) -> PoolReport {
    // Seul le schéma goldman ne garantit pas le GC de ses brins.
    if scheme.is_none_or(crate::Scheme::constrains_gc) {
        return pool_report(records, constraints);
    }
    let per_oligo: Vec<OligoReport> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let sequence = &record.sequence;
            OligoReport {
                index,
                length: sequence.len(),
                gc: gc_content(sequence),
                max_homopolymer: max_homopolymer(sequence),
                compliant: max_homopolymer(sequence) <= constraints.max_homopolymer,
            }
        })
        .collect();
    aggregate(&per_oligo, *constraints)
}

/// Agrège des statistiques d'oligos déjà calculées. Les longueisses nulles (séquences vides)
/// comptent comme oligos, mais n'entrent pas dans les bornes de longueur.
fn aggregate(per_oligo: &[OligoReport], constraints: DnaConstraints) -> PoolReport {
    let mut report = PoolReport {
        oligos: per_oligo.len(),
        nucleotides: 0,
        length_min: 0,
        length_max: 0,
        gc_min: 0.0,
        gc_mean: 0.0,
        gc_max: 0.0,
        max_homopolymer: 0,
        compliant: 0,
        constraints,
    };
    let mut gc_sum = 0.0;
    let mut non_empty = 0usize;
    for oligo in per_oligo {
        report.nucleotides += oligo.length as u64;
        report.compliant += usize::from(oligo.compliant);
        if oligo.length == 0 {
            continue;
        }
        non_empty += 1;
        gc_sum += oligo.gc;
        report.length_min = if non_empty == 1 {
            oligo.length
        } else {
            report.length_min.min(oligo.length)
        };
        report.length_max = report.length_max.max(oligo.length);
        report.gc_min = if non_empty == 1 {
            oligo.gc
        } else {
            report.gc_min.min(oligo.gc)
        };
        report.gc_max = if non_empty == 1 {
            oligo.gc
        } else {
            report.gc_max.max(oligo.gc)
        };
        report.max_homopolymer = report.max_homopolymer.max(oligo.max_homopolymer);
    }
    if non_empty > 0 {
        report.gc_mean = gc_sum / non_empty as f64;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fasta::Record;

    fn records(sequences: &[&str]) -> Vec<Record> {
        sequences
            .iter()
            .enumerate()
            .map(|(i, sequence)| Record {
                header: format!("r{i}"),
                sequence: sequence.as_bytes().to_vec(),
            })
            .collect()
    }

    #[test]
    fn pool_statistics() {
        let list = records(&["ACGTTGCAACGTTCGA", "AAAACCCCGGGGTTTT", "ACGT"]);
        let (report, per_oligo) = inspect(&list, &DnaConstraints::default());
        assert_eq!(report.oligos, 3);
        assert_eq!(report.nucleotides, 36);
        assert_eq!(report.length_min, 4);
        assert_eq!(report.length_max, 16);
        assert!((report.gc_min - 0.5).abs() < 1e-12);
        assert!((report.gc_max - 0.5).abs() < 1e-12);
        assert_eq!(report.max_homopolymer, 4);
        assert_eq!(report.compliant, 2);
        assert_eq!(per_oligo.len(), 3);
        assert_eq!(per_oligo[1].max_homopolymer, 4);
        assert!(!per_oligo[1].compliant);
        assert_eq!(per_oligo[2].length, 4);
    }

    #[test]
    fn empty_and_partly_empty_pools() {
        let constraints = DnaConstraints::default();
        let empty = pool_report(&[], &constraints);
        assert_eq!(
            empty,
            PoolReport {
                oligos: 0,
                nucleotides: 0,
                length_min: 0,
                length_max: 0,
                gc_min: 0.0,
                gc_mean: 0.0,
                gc_max: 0.0,
                max_homopolymer: 0,
                compliant: 0,
                constraints
            }
        );
        let report = pool_report(&records(&["", "ACGT"]), &constraints);
        assert_eq!(report.oligos, 2);
        assert_eq!(report.length_min, 4);
        assert_eq!(report.length_max, 4);
        assert!((report.gc_mean - 0.5).abs() < 1e-12);
        assert_eq!(report.compliant, 1);
    }
}
