//! Module de performance et optimisation

use rayon::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::fs;

/// Cache pour les opérations coûteuses
#[derive(Debug)]
pub struct PerformanceCache {
    cache: Mutex<lru::LruCache<u64, Vec<u8>>>,
}

impl Default for PerformanceCache {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl PerformanceCache {
    /// Crée un nouveau cache avec une capacité donnée
    pub fn new(capacity: usize) -> Self {
        // Garantir au moins 1 pour éviter panic sur unwrap
        let cap = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1).unwrap());
        Self {
            cache: Mutex::new(lru::LruCache::new(cap)),
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

/// Cache hybride mémoire/disque pour les opérations coûteuses
#[derive(Debug)]
pub struct HybridCache {
    memory_cache: Arc<PerformanceCache>,
    disk_cache_enabled: bool,
    cache_dir: Mutex<Option<PathBuf>>,
    max_disk_size: usize, // en octets
}

impl HybridCache {
    /// Crée un nouveau cache hybride
    pub fn new(memory_capacity: usize, disk_cache_enabled: bool, cache_dir: Option<PathBuf>, max_disk_size: usize) -> Self {
        Self {
            memory_cache: Arc::new(PerformanceCache::new(memory_capacity)),
            disk_cache_enabled,
            cache_dir: Mutex::new(cache_dir),
            max_disk_size,
        }
    }

    /// Initialise le cache disque
    pub fn initialize_disk_cache(&self, cache_dir: PathBuf) -> crate::error::Result<()> {
        let mut dir_guard = self.cache_dir.lock();
        
        // Créer le répertoire s'il n'existe pas
        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)
                .map_err(|e| crate::error::DnaError::Io(e))?;
        }
        
        *dir_guard = Some(cache_dir);
        Ok(())
    }

    /// Génère un nom de fichier pour une clé
    fn get_cache_file_name(&self, key: u64) -> Option<PathBuf> {
        let dir_guard = self.cache_dir.lock();
        dir_guard.as_ref().map(|dir| dir.join(format!("{:016x}.cache", key)))
    }

    /// Ajoute un élément au cache (mémoire et disque)
    pub fn insert(&self, key: u64, value: Vec<u8>) -> crate::error::Result<()> {
        // Ajouter à la mémoire
        self.memory_cache.insert(key, value.clone());
        
        // Ajouter au disque si activé
        if self.disk_cache_enabled {
            if let Some(file_path) = self.get_cache_file_name(key) {
                // Sérialiser et écrire sur le disque
                let serialized = bincode::serialize(&value)
                    .map_err(|e| crate::error::DnaError::Serialization(e.to_string()))?;
                
                fs::write(&file_path, serialized)
                    .map_err(|e| crate::error::DnaError::Io(e))?;
                
                // Vérifier et nettoyer si nécessaire
                self.cleanup_disk_cache()?;
            }
        }
        
        Ok(())
    }

    /// Récupère un élément du cache (d'abord mémoire, puis disque)
    pub fn get(&self, key: u64) -> Option<Vec<u8>> {
        // D'abord vérifier la mémoire
        if let Some(value) = self.memory_cache.get(key) {
            return Some(value);
        }
        
        // Puis vérifier le disque si activé
        if self.disk_cache_enabled {
            if let Some(file_path) = self.get_cache_file_name(key) {
                if file_path.exists() {
                    if let Ok(serialized) = fs::read(&file_path) {
                        if let Ok(value) = bincode::deserialize::<Vec<u8>>(&serialized) {
                            // Mettre à jour le cache mémoire
                            self.memory_cache.insert(key, value.clone());
                            return Some(value);
                        }
                    }
                }
            }
        }
        
        None
    }

    /// Nettoie le cache
    pub fn clear(&self) -> crate::error::Result<()> {
        self.memory_cache.clear();
        
        if self.disk_cache_enabled {
            if let Some(dir) = self.cache_dir.lock().as_ref() {
                if dir.exists() {
                    for entry in fs::read_dir(dir)? {
                        let entry = entry?;
                        if entry.file_type()?.is_file() {
                            fs::remove_file(entry.path())?;
                        }
                    }
                }
            }
        }
        
        Ok(())
    }

    /// Nettoie le cache disque si nécessaire
    fn cleanup_disk_cache(&self) -> crate::error::Result<()> {
        if !self.disk_cache_enabled {
            return Ok(());
        }
        
        if let Some(dir) = self.cache_dir.lock().as_ref() {
            if !dir.exists() {
                return Ok(());
            }
            
            let mut total_size = 0usize;
            let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
            
            // Calculer la taille totale et collecter les fichiers
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let metadata = entry.metadata()?;
                    let file_size = metadata.len() as usize;
                    total_size += file_size;
                    
                    if let Ok(modified) = entry.metadata()?.modified() {
                        files.push((entry.path(), file_size as u64, modified));
                    }
                }
            }
            
            // Nettoyer si nécessaire
            if total_size > self.max_disk_size {
                // Trier par date de modification (les plus anciens d'abord)
                files.sort_by(|a, b| a.2.cmp(&b.2));
                
                // Supprimer les fichiers jusqu'à ce que nous soyons sous la limite
                for (path, file_size, _) in files {
                    if total_size <= self.max_disk_size {
                        break;
                    }
                    
                    fs::remove_file(&path)?;
                    total_size -= file_size as usize;
                }
            }
        }
        
        Ok(())
    }

    /// Retourne la taille actuelle du cache disque
    pub fn disk_cache_size(&self) -> crate::error::Result<usize> {
        if !self.disk_cache_enabled {
            return Ok(0);
        }
        
        let mut total_size = 0usize;
        
        if let Some(dir) = self.cache_dir.lock().as_ref() {
            if dir.exists() {
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
                        total_size += entry.metadata()?.len() as usize;
                    }
                }
            }
        }
        
        Ok(total_size)
    }

    /// Retourne le nombre d'entrées dans le cache mémoire
    pub fn memory_cache_len(&self) -> usize {
        self.memory_cache.cache.lock().len()
    }

    /// Retourne le cache mémoire sous-jacent
    pub fn memory_cache(&self) -> Arc<PerformanceCache> {
        self.memory_cache.clone()
    }
}

/// Stratégie de cache
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStrategy {
    /// Cache mémoire uniquement
    MemoryOnly,
    /// Cache disque uniquement
    DiskOnly,
    /// Cache hybride (mémoire d'abord, puis disque)
    Hybrid,
    /// Pas de cache
    None,
}

/// Gestionnaire de cache avancé
#[derive(Debug)]
pub struct AdvancedCacheManager {
    strategy: CacheStrategy,
    memory_cache: Option<Arc<PerformanceCache>>,
    hybrid_cache: Option<Arc<HybridCache>>,
}

impl AdvancedCacheManager {
    /// Crée un nouveau gestionnaire de cache
    pub fn new(strategy: CacheStrategy, memory_capacity: usize, 
                _disk_enabled: bool, cache_dir: Option<PathBuf>, max_disk_size: usize) 
                -> crate::error::Result<Self> {
        
        let (memory_cache, hybrid_cache) = match strategy {
            CacheStrategy::MemoryOnly => {
                (Some(Arc::new(PerformanceCache::new(memory_capacity))), None)
            }
            CacheStrategy::DiskOnly => {
                let cache_dir_clone = cache_dir.clone();
                let hybrid_cache = HybridCache::new(1, true, cache_dir, max_disk_size);
                if let Some(dir) = cache_dir_clone {
                    hybrid_cache.initialize_disk_cache(dir)?;
                }
                (None, Some(Arc::new(hybrid_cache)))
            }
            CacheStrategy::Hybrid => {
                let cache_dir_clone = cache_dir.clone();
                let hybrid_cache = HybridCache::new(memory_capacity, true, cache_dir, max_disk_size);
                if let Some(dir) = cache_dir_clone {
                    hybrid_cache.initialize_disk_cache(dir)?;
                }
                (None, Some(Arc::new(hybrid_cache)))
            }
            CacheStrategy::None => (None, None),
        };
        
        Ok(Self {
            strategy,
            memory_cache,
            hybrid_cache,
        })
    }

    /// Insère une valeur dans le cache
    pub fn insert(&self, key: u64, value: Vec<u8>) -> crate::error::Result<()> {
        match self.strategy {
            CacheStrategy::MemoryOnly => {
                if let Some(cache) = &self.memory_cache {
                    cache.insert(key, value);
                }
            }
            CacheStrategy::DiskOnly | CacheStrategy::Hybrid => {
                if let Some(cache) = &self.hybrid_cache {
                    cache.insert(key, value)?;
                }
            }
            CacheStrategy::None => {}
        }
        
        Ok(())
    }

    /// Récupère une valeur du cache
    pub fn get(&self, key: u64) -> Option<Vec<u8>> {
        match self.strategy {
            CacheStrategy::MemoryOnly => {
                self.memory_cache.as_ref().and_then(|cache| cache.get(key))
            }
            CacheStrategy::DiskOnly | CacheStrategy::Hybrid => {
                self.hybrid_cache.as_ref().and_then(|cache| cache.get(key))
            }
            CacheStrategy::None => None,
        }
    }

    /// Nettoie le cache
    pub fn clear(&self) -> crate::error::Result<()> {
        match self.strategy {
            CacheStrategy::MemoryOnly => {
                if let Some(cache) = &self.memory_cache {
                    cache.clear();
                }
            }
            CacheStrategy::DiskOnly | CacheStrategy::Hybrid => {
                if let Some(cache) = &self.hybrid_cache {
                    cache.clear()?;
                }
            }
            CacheStrategy::None => {}
        }
        
        Ok(())
    }

    /// Retourne la stratégie de cache
    pub fn strategy(&self) -> CacheStrategy {
        self.strategy
    }

    /// Retourne la taille du cache mémoire
    pub fn memory_cache_size(&self) -> Option<usize> {
        match self.strategy {
            CacheStrategy::MemoryOnly => {
                self.memory_cache.as_ref().map(|cache| cache.cache.lock().len())
            }
            CacheStrategy::Hybrid => {
                self.hybrid_cache.as_ref().map(|cache| cache.memory_cache_len())
            }
            _ => None,
        }
    }

    /// Retourne la taille du cache disque
    pub fn disk_cache_size(&self) -> crate::error::Result<Option<usize>> {
        match self.strategy {
            CacheStrategy::DiskOnly | CacheStrategy::Hybrid => {
                if let Some(cache) = &self.hybrid_cache {
                    Ok(Some(cache.disk_cache_size()?))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
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
                    .map(&operation)
                    .collect::<Vec<_>>()
            })
            .flatten()
            .collect()
    }

    /// Retourne le cache
    pub fn cache(&self) -> Arc<PerformanceCache> {
        self.cache.clone()
    }
}