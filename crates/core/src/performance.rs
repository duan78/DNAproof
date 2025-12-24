//! Module de performance et optimisation

use rayon::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;

/// Cache pour les opérations coûteuses
#[derive(Debug, Default)]
pub struct PerformanceCache {
    cache: Mutex<lru::LruCache<u64, Vec<u8>>>,
}

impl PerformanceCache {
    /// Crée un nouveau cache avec une capacité donnée
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: Mutex::new(lru::LruCache::new(capacity)),
        }
    }

    /// Ajoute un élément au cache
    pub fn insert(&self, key: u64, value: Vec<u8>) {
        self.cache.lock().put(key, value);
    }

    /// Récupère un élément du cache
    pub fn get(&self, key: u64) -> Option<Vec<u8>> {
        self.cache.lock().get(&key).cloned()
    }

    /// Nettoie le cache
    pub fn clear(&self) {
        self.cache.lock().clear();
    }
}

/// Optimiseur de performance pour les opérations parallèles
pub struct PerformanceOptimizer {
    cache: Arc<PerformanceCache>,
    parallelism: usize,
}

impl PerformanceOptimizer {
    /// Crée un nouvel optimiseur
    pub fn new(cache_size: usize, parallelism: usize) -> Self {
        Self {
            cache: Arc::new(PerformanceCache::new(cache_size)),
            parallelism,
        }
    }

    /// Exécute une opération en parallèle
    pub fn parallel_operation<T, F>(&self, data: &[T], operation: F) -> Vec<Vec<u8>>
    where
        T: Sync + Send,
        F: Fn(&T) -> Vec<u8> + Sync + Send,
    {
        data.par_chunks(self.parallelism)
            .map(|chunk| {
                chunk.iter()
                    .map(|item| operation(item))
                    .collect()
            })
            .flatten()
            .collect()
    }

    /// Retourne le cache
    pub fn cache(&self) -> Arc<PerformanceCache> {
        self.cache.clone()
    }
}