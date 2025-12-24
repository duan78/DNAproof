//! Indexation et recherche de séquences

use adn_core::DnaSequence;
use std::collections::HashMap;

/// Index de séquences pour la recherche rapide
#[derive(Debug)]
pub struct SequenceIndex {
    /// Map sequence_id -> méta
    by_id: HashMap<String, SequenceMeta>,

    /// Index par fichier d'origine
    by_file: HashMap<String, Vec<String>>,

    /// Index par seed
    by_seed: HashMap<u64, String>,
}

/// Métadonnées d'indexation
#[derive(Debug, Clone)]
struct SequenceMeta {
    id: String,
    file: String,
    seed: u64,
}

impl SequenceIndex {
    /// Crée un nouvel index
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_file: HashMap::new(),
            by_seed: HashMap::new(),
        }
    }

    /// Insère une séquence dans l'index
    pub fn insert(&mut self, sequence: &DnaSequence) {
        let id = sequence.id.to_string();
        let file = sequence.metadata.original_file.clone();
        let seed = sequence.metadata.seed;

        let meta = SequenceMeta {
            id: id.clone(),
            file: file.clone(),
            seed,
        };

        // Index par ID
        self.by_id.insert(id.clone(), meta.clone());

        // Index par fichier
        self.by_file.entry(file).or_insert_with(Vec::new).push(id.clone());

        // Index par seed
        self.by_seed.insert(seed, id);
    }

    /// Supprime une séquence de l'index
    pub fn remove(&mut self, sequence: &DnaSequence) {
        let id = sequence.id.to_string();
        let file = sequence.metadata.original_file.clone();
        let seed = sequence.metadata.seed;

        self.by_id.remove(&id);
        self.by_seed.remove(&seed);

        if let Some(entries) = self.by_file.get_mut(&file) {
            entries.retain(|e| e != &id);
        }
    }

    /// Recherche des séquences par similarité de chaîne
    pub fn search(&self, query: &str, _threshold: f64) -> Vec<String> {
        let mut results = Vec::new();

        // Recherche simple par sous-chaîne dans le nom de fichier
        for (file, ids) in &self.by_file {
            if file.contains(query) {
                results.extend(ids.clone());
            }
        }

        results
    }

    /// Retourne les IDs de séquences pour un fichier donné
    pub fn by_file(&self, file: &str) -> Vec<String> {
        self.by_file.get(file).cloned().unwrap_or_default()
    }

    /// Retourne l'ID pour un seed donné
    pub fn by_seed(&self, seed: u64) -> Option<&String> {
        self.by_seed.get(&seed)
    }

    /// Vide l'index
    pub fn clear(&mut self) {
        self.by_id.clear();
        self.by_file.clear();
        self.by_seed.clear();
    }
}

impl Default for SequenceIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Résultat de recherche
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub file: String,
    pub score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use adn_core::{IupacBase, DnaSequence};

    #[test]
    fn test_index_creation() {
        let index = SequenceIndex::new();
        assert!(index.by_id.is_empty());
    }

    #[test]
    fn test_insert_and_retrieve() {
        let mut index = SequenceIndex::new();

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test.txt".to_string(), 0, 4, 42);

        index.insert(&seq);

        let ids = index.by_file("test.txt");
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_search() {
        let mut index = SequenceIndex::new();

        let bases = vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T];
        let seq = DnaSequence::new(bases, "test_file.txt".to_string(), 0, 4, 42);

        index.insert(&seq);

        let results = index.search("test", 0.5);
        assert_eq!(results.len(), 1);
    }
}
