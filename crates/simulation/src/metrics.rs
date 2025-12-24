//! Métriques de simulation

use serde::{Deserialize, Serialize};

/// Métriques collectées pendant une simulation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimulationMetrics {
    /// Nombre total de bases
    pub total_bases: usize,

    /// Nombre de substitutions
    pub substitutions: usize,

    /// Nombre d'insertions
    pub insertions: usize,

    /// Nombre de délétions
    pub deletions: usize,

    /// Nombre total de bases affectées
    pub affected_bases: usize,
}

impl SimulationMetrics {
    /// Crée de nouvelles métriques vides
    pub fn new() -> Self {
        Self::default()
    }

    /// Taux d'erreur total
    pub fn error_rate(&self) -> f64 {
        if self.total_bases == 0 {
            return 0.0;
        }
        self.affected_bases as f64 / self.total_bases as f64
    }

    /// Taux de substitution
    pub fn substitution_rate(&self) -> f64 {
        if self.total_bases == 0 {
            return 0.0;
        }
        self.substitutions as f64 / self.total_bases as f64
    }

    /// Taux d'insertion
    pub fn insertion_rate(&self) -> f64 {
        if self.total_bases == 0 {
            return 0.0;
        }
        self.insertions as f64 / self.total_bases as f64
    }

    /// Taux de délétion
    pub fn deletion_rate(&self) -> f64 {
        if self.total_bases == 0 {
            return 0.0;
        }
        self.deletions as f64 / self.total_bases as f64
    }

    /// Formate les métriques en tableau
    pub fn format_table(&self) -> String {
        format!(
            "┌────────────────────────────────────┐\n\
             │ Métriques de Simulation            │\n\
             ├────────────────────────────────────┤\n\
             │ Bases totales    : {:>6}          │\n\
             │ Substitutions    : {:>6} ({:>5.1}%)  │\n\
             │ Insertions       : {:>6} ({:>5.1}%)  │\n\
             │ Délétions        : {:>6} ({:>5.1}%)  │\n\
             │ Taux d'erreur    : {:>6.1}%        │\n\
             └────────────────────────────────────┘",
            self.total_bases,
            self.substitutions,
            100.0 * self.substitution_rate(),
            self.insertions,
            100.0 * self.insertion_rate(),
            self.deletions,
            100.0 * self.deletion_rate(),
            100.0 * self.error_rate()
        )
    }
}

/// Collecteur de métriques pour plusieurs simulations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsCollector {
    metrics: Vec<SimulationMetrics>,
}

impl MetricsCollector {
    /// Crée un nouveau collecteur
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute des métriques
    pub fn add(&mut self, metrics: SimulationMetrics) {
        self.metrics.push(metrics);
    }

    /// Retourne les métriques moyennes
    pub fn average(&self) -> SimulationMetrics {
        if self.metrics.is_empty() {
            return SimulationMetrics::new();
        }

        let n = self.metrics.len();

        SimulationMetrics {
            total_bases: self.metrics.iter().map(|m| m.total_bases).sum::<usize>() / n,
            substitutions: self.metrics.iter().map(|m| m.substitutions).sum::<usize>() / n,
            insertions: self.metrics.iter().map(|m| m.insertions).sum::<usize>() / n,
            deletions: self.metrics.iter().map(|m| m.deletions).sum::<usize>() / n,
            affected_bases: self.metrics.iter().map(|m| m.affected_bases).sum::<usize>() / n,
        }
    }

    /// Retourne les métriques minimales
    pub fn min(&self) -> SimulationMetrics {
        if self.metrics.is_empty() {
            return SimulationMetrics::new();
        }

        SimulationMetrics {
            total_bases: self.metrics.iter().map(|m| m.total_bases).min().unwrap_or(0),
            substitutions: self.metrics.iter().map(|m| m.substitutions).min().unwrap_or(0),
            insertions: self.metrics.iter().map(|m| m.insertions).min().unwrap_or(0),
            deletions: self.metrics.iter().map(|m| m.deletions).min().unwrap_or(0),
            affected_bases: self.metrics.iter().map(|m| m.affected_bases).min().unwrap_or(0),
        }
    }

    /// Retourne les métriques maximales
    pub fn max(&self) -> SimulationMetrics {
        if self.metrics.is_empty() {
            return SimulationMetrics::new();
        }

        SimulationMetrics {
            total_bases: self.metrics.iter().map(|m| m.total_bases).max().unwrap_or(0),
            substitutions: self.metrics.iter().map(|m| m.substitutions).max().unwrap_or(0),
            insertions: self.metrics.iter().map(|m| m.insertions).max().unwrap_or(0),
            deletions: self.metrics.iter().map(|m| m.deletions).max().unwrap_or(0),
            affected_bases: self.metrics.iter().map(|m| m.affected_bases).max().unwrap_or(0),
        }
    }

    /// Retourne l'écart type
    pub fn std_dev(&self) -> SimulationMetrics {
        if self.metrics.len() < 2 {
            return SimulationMetrics::new();
        }

        let _avg = self.average();
        let n = self.metrics.len();

        let variance = |values: Vec<f64>| -> f64 {
            let mean = values.iter().sum::<f64>() / n as f64;
            values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n as f64
        };

        let sub_values: Vec<f64> = self.metrics.iter().map(|m| m.substitutions as f64).collect();
        let ins_values: Vec<f64> = self.metrics.iter().map(|m| m.insertions as f64).collect();
        let del_values: Vec<f64> = self.metrics.iter().map(|m| m.deletions as f64).collect();

        SimulationMetrics {
            total_bases: 0,
            substitutions: variance(sub_values).sqrt() as usize,
            insertions: variance(ins_values).sqrt() as usize,
            deletions: variance(del_values).sqrt() as usize,
            affected_bases: 0,
        }
    }

    /// Nombre de simulations
    pub fn len(&self) -> usize {
        self.metrics.len()
    }

    /// Vérifie si vide
    pub fn is_empty(&self) -> bool {
        self.metrics.is_empty()
    }

    /// Vide le collecteur
    pub fn clear(&mut self) {
        self.metrics.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = SimulationMetrics::new();
        assert_eq!(metrics.total_bases, 0);
    }

    #[test]
    fn test_error_rate() {
        let mut metrics = SimulationMetrics::new();
        metrics.total_bases = 100;
        metrics.substitutions = 5;
        metrics.affected_bases = 5;

        assert!((metrics.error_rate() - 0.05).abs() < 1e-9);
    }

    #[test]
    fn test_collector() {
        let mut collector = MetricsCollector::new();

        let mut metrics1 = SimulationMetrics::new();
        metrics1.total_bases = 100;
        metrics1.substitutions = 10;

        let mut metrics2 = SimulationMetrics::new();
        metrics2.total_bases = 100;
        metrics2.substitutions = 20;

        collector.add(metrics1);
        collector.add(metrics2);

        let avg = collector.average();
        assert_eq!(avg.substitutions, 15);
    }
}
