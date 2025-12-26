//! Pool de séquences ADN avec gestion de stockage

use crate::index::SequenceIndex;
use adn_core::{DnaConstraints, DnaSequence, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Configuration du pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    /// Contraintes ADN pour validation
    pub constraints: DnaConstraints,

    /// Taille maximale du pool (nombre de séquences)
    pub max_size: usize,

    /// Activer la persistance
    pub persistence_enabled: bool,

    /// Répertoire de persistance
    pub persistence_dir: Option<String>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            constraints: DnaConstraints::default(),
            max_size: 100000,
            persistence_enabled: false,
            persistence_dir: None,
        }
    }
}

/// Pool de séquences ADN
#[derive(Debug)]
pub struct DnaPool {
    sequences: HashMap<String, DnaSequence>,
    index: SequenceIndex,
    config: PoolConfig,
}

impl DnaPool {
    /// Crée un nouveau pool
    pub fn new(config: PoolConfig) -> Self {
        Self {
            sequences: HashMap::new(),
            index: SequenceIndex::new(),
            config,
        }
    }

    /// Ajoute une séquence au pool
    pub fn insert(&mut self, sequence: DnaSequence) -> Result<()> {
        // Valider les contraintes
        sequence.validate(&self.config.constraints)?;

        let id = sequence.id.to_string();

        // Insérer dans le pool
        self.sequences.insert(id.clone(), sequence.clone());

        // Mettre à jour l'index
        self.index.insert(&sequence);

        Ok(())
    }

    /// Récupère une séquence par ID
    pub fn get(&self, id: &str) -> Option<&DnaSequence> {
        self.sequences.get(id)
    }

    /// Supprime une séquence du pool
    pub fn remove(&mut self, id: &str) -> Option<DnaSequence> {
        let seq = self.sequences.remove(id)?;
        self.index.remove(&seq);
        Some(seq)
    }

    /// Retourne toutes les séquences
    pub fn all(&self) -> Vec<&DnaSequence> {
        self.sequences.values().collect()
    }

    /// Retourne le nombre de séquences
    pub fn len(&self) -> usize {
        self.sequences.len()
    }

    /// Vérifie si le pool est vide
    pub fn is_empty(&self) -> bool {
        self.sequences.is_empty()
    }

    /// Recherche des séquences par similarité
    pub fn search(&self, query: &str, threshold: f64) -> Vec<&DnaSequence> {
        self.index.search(query, threshold)
            .into_iter()
            .filter_map(|id| self.sequences.get(&id))
            .collect()
    }

    /// Vide le pool
    pub fn clear(&mut self) {
        self.sequences.clear();
        self.index = SequenceIndex::new();
    }

    /// Sauvegarde le pool sur disque
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.sequences)
            .map_err(|e| adn_core::error::DnaError::Serialization(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Charge un pool depuis disque
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let json = std::fs::read_to_string(path)?;
        let sequences: HashMap<String, DnaSequence> = serde_json::from_str(&json)
            .map_err(|e| adn_core::error::DnaError::Serialization(e.to_string()))?;

        self.sequences = sequences;

        // Reconstruire l'index
        self.index = SequenceIndex::new();
        for seq in self.sequences.values() {
            self.index.insert(seq);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adn_core::IupacBase;

    #[test]
    fn test_pool_creation() {
        let config = PoolConfig::default();
        let pool = DnaPool::new(config);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_insert_and_get() {
        let config = PoolConfig::default();
        let mut pool = DnaPool::new(config);

        let bases = vec![
            IupacBase::A,
            IupacBase::C,
            IupacBase::G,
            IupacBase::T,
        ];

        let seq = DnaSequence::new(
            bases,
            "test.txt".to_string(),
            0,
            4,
            42,
        );

        let id = seq.id.to_string();
        pool.insert(seq).unwrap();

        assert_eq!(pool.len(), 1);
        assert!(pool.get(&id).is_some());
    }

    #[test]
    fn test_remove() {
        let config = PoolConfig::default();
        let mut pool = DnaPool::new(config);

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        let id = seq.id.to_string();
        pool.insert(seq).unwrap();

        pool.remove(&id);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_clear() {
        let config = PoolConfig::default();
        let mut pool = DnaPool::new(config);

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        pool.insert(seq).unwrap();
        pool.clear();

        assert!(pool.is_empty());
    }
}
