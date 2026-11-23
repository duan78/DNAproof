//! Tâches d'encodage et de décodage : suivies en mémoire, écrites sur disque, et recopiées en
//! base de données quand elle est activée.

use adn_storage::{ArchiveInfo, SequenceRepository};
use jiff::Timestamp;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, PoisonError, RwLock};
use tokio::sync::Semaphore;
use tracing::{info, warn};
use uuid::Uuid;

/// Ce que fait une tâche.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    /// Un fichier devient une archive FASTA.
    Encode,
    /// Une archive FASTA rend un fichier.
    Decode,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Encode => "encodage",
            JobKind::Decode => "décodage",
        }
    }
}

/// Où en est une tâche.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// En attente d'un emplacement de travail.
    Queued,
    Running,
    Complete,
    Failed,
}

/// Une tâche, telle qu'elle est rendue par l'API.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    /// Étape en cours, pour une tâche en attente ou qui tourne.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Fichier envoyé par l'utilisateur.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    /// Nom proposé pour le téléchargement, une fois la tâche terminée.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    /// Empreinte SHA-256 du fichier rendu, en hexadécimal. À l'encodage, c'est celle du fichier
    /// d'origine, que l'archive embarque.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Taille du fichier rendu, en octets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Mesures propres au schéma.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Job {
    fn new(kind: JobKind, source_name: Option<String>) -> Self {
        let now = Timestamp::now();
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            status: JobStatus::Queued,
            stage: Some("en attente".to_string()),
            source_name,
            file_name: None,
            download_url: None,
            sha256: None,
            size: None,
            stats: None,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Vrai s'il n'y a plus rien à attendre de cette tâche.
    pub fn is_finished(&self) -> bool {
        matches!(self.status, JobStatus::Complete | JobStatus::Failed)
    }
}

/// Résultat d'une tâche qui a réussi.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// Nom proposé pour le téléchargement.
    pub file_name: String,
    pub sha256: String,
    /// Taille du fichier écrit, en octets.
    pub size: u64,
    pub stats: serde_json::Value,
}

/// Tâches d'un serveur.
pub struct JobRegistry {
    jobs: RwLock<HashMap<String, Job>>,
    directory: PathBuf,
    repository: Option<Arc<SequenceRepository>>,
    slots: Arc<Semaphore>,
}

impl JobRegistry {
    /// Prépare le suivi des tâches, et le dossier de leurs résultats.
    pub fn new(
        directory: impl Into<PathBuf>,
        concurrent_jobs: usize,
        repository: Option<SequenceRepository>,
    ) -> std::io::Result<Self> {
        let directory = directory.into();
        std::fs::create_dir_all(&directory)?;
        Ok(Self {
            jobs: RwLock::new(HashMap::new()),
            directory,
            repository: repository.map(Arc::new),
            slots: Arc::new(Semaphore::new(concurrent_jobs.max(1))),
        })
    }

    /// Chemin du fichier produit par une tâche. `id` doit avoir été validé.
    pub fn result_path(&self, id: &str, kind: JobKind) -> PathBuf {
        let extension = match kind {
            JobKind::Encode => "fasta",
            JobKind::Decode => "bin",
        };
        self.directory.join(format!("{id}.{extension}"))
    }

    /// La base de données, si elle est activée.
    pub fn database(&self) -> Option<&SequenceRepository> {
        self.repository.as_deref()
    }

    /// Dépose une tâche en attente et la rend.
    pub fn submit(&self, kind: JobKind, source_name: Option<String>) -> Job {
        let job = Job::new(kind, source_name);
        self.write_jobs().insert(job.id.clone(), job.clone());
        job
    }

    /// Une tâche, par son identifiant.
    pub fn get(&self, id: &str) -> Option<Job> {
        self.read_jobs().get(id).cloned()
    }

    /// Les tâches, de la plus récente à la plus ancienne.
    pub fn list(&self) -> Vec<Job> {
        let mut jobs: Vec<Job> = self.read_jobs().values().cloned().collect();
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        jobs
    }

    /// Nombre de tâches dans chaque état.
    pub fn counts(&self) -> JobCounts {
        let mut counts = JobCounts::default();
        for job in self.read_jobs().values() {
            match job.status {
                JobStatus::Queued => counts.queued += 1,
                JobStatus::Running => counts.running += 1,
                JobStatus::Complete => counts.complete += 1,
                JobStatus::Failed => counts.failed += 1,
            }
        }
        counts
    }

    /// Nombre de tâches qui attendent ou qui travaillent.
    pub fn pending(&self) -> usize {
        self.read_jobs()
            .values()
            .filter(|job| !job.is_finished())
            .count()
    }

    /// Signale qu'une tâche travaille, et à quelle étape elle en est.
    pub fn set_stage(&self, id: &str, stage: &str) {
        self.update(id, |job| {
            job.status = JobStatus::Running;
            job.stage = Some(stage.to_string());
        });
    }

    /// Termine une tâche, par un succès ou par un échec.
    pub fn finish(&self, id: &str, outcome: Result<Outcome, String>) {
        self.update(id, |job| {
            job.stage = None;
            match outcome {
                Ok(outcome) => {
                    job.status = JobStatus::Complete;
                    job.file_name = Some(outcome.file_name);
                    job.download_url = Some(match job.kind {
                        JobKind::Encode => format!("/download/fasta/{id}"),
                        JobKind::Decode => format!("/download/{id}"),
                    });
                    job.sha256 = Some(outcome.sha256);
                    job.size = Some(outcome.size);
                    job.stats = Some(outcome.stats);
                }
                Err(error) => {
                    warn!("Tâche {} ({}) échouée : {error}", id, job.kind.label());
                    job.status = JobStatus::Failed;
                    job.error = Some(error);
                    // Un résultat à moitié écrit ne doit pas rester téléchargeable.
                    let _ = std::fs::remove_file(self.result_path(id, job.kind));
                }
            }
        });
    }

    /// Retire une tâche terminée et efface ses fichiers.
    ///
    /// Une tâche en attente ou en cours n'est pas retirée. Il n'y a pas d'annulation : elle
    /// continuerait, et son résultat, écrit plus tard, ne serait plus ni listé ni effaçable.
    pub fn remove(&self, id: &str) -> Removal {
        let mut jobs = self.write_jobs();
        match jobs.get(id) {
            None => return Removal::Unknown,
            Some(job) if !job.is_finished() => return Removal::Busy,
            Some(_) => {
                jobs.remove(id);
            }
        }
        for kind in [JobKind::Encode, JobKind::Decode] {
            let _ = std::fs::remove_file(self.result_path(id, kind));
        }
        Removal::Removed
    }

    /// Les tâches terminées plus vieilles que la durée de conservation, qui vaut `0` pour
    /// garder les résultats sans limite.
    pub fn expired(&self, now: Timestamp, retention_minutes: u64) -> Vec<String> {
        if retention_minutes == 0 {
            return Vec::new();
        }
        let cutoff = now.as_second() - (retention_minutes as i64) * 60;
        self.read_jobs()
            .iter()
            .filter(|(_, job)| job.is_finished() && job.updated_at.as_second() < cutoff)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Efface en base l'archive d'une tâche retirée.
    pub async fn forget_archive(&self, id: &str) -> Result<(), String> {
        match &self.repository {
            Some(repository) => repository
                .delete_archive(id)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// Range une archive en base. Bloque le thread appelant : réservé aux threads de travail.
    pub fn store_archive_blocking(
        &self,
        info: &ArchiveInfo,
        oligos: &[Vec<u8>],
    ) -> Result<(), String> {
        match &self.repository {
            Some(repository) => tokio::runtime::Handle::current()
                .block_on(repository.save_archive(info, oligos))
                .map(|_| ())
                .map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// Retrouve les archives de la base après un redémarrage. Une archive dont le fichier a
    /// disparu est réécrite depuis ses oligos. Rend le nombre de tâches retrouvées.
    pub async fn restore(&self) -> Result<usize, String> {
        let Some(repository) = &self.repository else {
            return Ok(0);
        };
        let archives = repository.archives().await.map_err(|e| e.to_string())?;
        let mut restored = 0;
        for archive in archives {
            let id = archive.info.id.as_str();
            // L'identifiant devient un chemin de fichier : seuls ceux que le serveur a produits
            // sont repris.
            if Uuid::parse_str(id).is_err() {
                warn!("Archive {id} ignorée : l'identifiant n'est pas celui d'une tâche");
                continue;
            }
            let path = self.result_path(id, JobKind::Encode);
            if !path.is_file() {
                let oligos = repository
                    .archive_sequences(id)
                    .await
                    .map_err(|e| e.to_string())?;
                if oligos.is_empty() {
                    warn!("Archive {id} sans oligos en base : elle est oubliée");
                    repository
                        .delete_archive(id)
                        .await
                        .map_err(|e| e.to_string())?;
                    continue;
                }
                rebuild_archive(&path, &oligos, &archive.info.scheme)?;
                info!("Archive {id} réécrite depuis la base de données");
            }
            let size = std::fs::metadata(&path)
                .map_err(|e| format!("taille de {} : {e}", path.display()))?
                .len();
            let mut job = Job::new(JobKind::Encode, Some(archive.info.file_name.clone()));
            job.id = id.to_string();
            job.status = JobStatus::Complete;
            job.stage = None;
            job.file_name = Some(crate::pipeline::archive_download_name(
                &archive.info.file_name,
            ));
            job.download_url = Some(format!("/download/fasta/{id}"));
            job.sha256 = Some(archive.info.sha256.clone());
            job.size = Some(size);
            job.stats = Some(
                serde_json::from_str(&archive.info.stats)
                    .unwrap_or_else(|_| serde_json::json!({ "oligos": archive.oligos })),
            );
            let created =
                Timestamp::from_str(&archive.created_at).unwrap_or_else(|_| Timestamp::now());
            job.created_at = created;
            job.updated_at = created;
            self.write_jobs().insert(id.to_string(), job);
            restored += 1;
        }
        Ok(restored)
    }

    fn write_jobs(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Job>> {
        self.jobs.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_jobs(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Job>> {
        self.jobs.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Job)) {
        let mut jobs = self.write_jobs();
        if let Some(job) = jobs.get_mut(id) {
            change(job);
            job.updated_at = Timestamp::now();
        }
    }
}

/// Issue du retrait d'une tâche.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// Retirée, ses fichiers effacés.
    Removed,
    /// En attente ou en cours : elle continue.
    Busy,
    /// Aucune tâche de ce nom.
    Unknown,
}

/// Nombre de tâches dans chaque état.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct JobCounts {
    pub queued: usize,
    pub running: usize,
    pub complete: usize,
    pub failed: usize,
}

/// Mène une tâche sur un thread de travail. Au plus `concurrent_jobs` tâches travaillent à la
/// fois : les autres attendent leur tour, et le serveur continue de répondre.
pub fn launch(
    registry: Arc<JobRegistry>,
    job_id: &str,
    work: impl FnOnce(&JobRegistry, &str) -> Result<Outcome, String> + Send + 'static,
) {
    let job_id = job_id.to_string();
    tokio::spawn(async move {
        let outcome = match Arc::clone(&registry.slots).acquire_owned().await {
            Ok(_permit) => {
                registry.set_stage(&job_id, "démarrage");
                let id = job_id.clone();
                let worker = Arc::clone(&registry);
                tokio::task::spawn_blocking(move || work(&worker, &id))
                    .await
                    .unwrap_or_else(|error| Err(format!("tâche interrompue : {error}")))
            }
            // Le sémaphore n'est jamais fermé ; le cas est prévu pour ne rien ignorer.
            Err(_) => Err("le serveur s'arrête".to_string()),
        };
        registry.finish(&job_id, outcome);
    });
}

/// Écrit une archive à partir des oligos de la base. Les en-têtes retrouvent leur forme
/// habituelle : les manifestes, porteurs de leur étiquette en clair, viennent en tête.
fn rebuild_archive(path: &Path, oligos: &[Vec<u8>], scheme: &str) -> Result<(), String> {
    let scheme = adn_core::Scheme::from_name(scheme)
        .ok_or_else(|| format!("schéma inconnu en base : {scheme}"))?;
    let tag = adn_core::codec::manifest::tag_dna();
    let manifests = oligos
        .iter()
        .take_while(|oligo| adn_core::codec::manifest::looks_like_manifest(oligo, &tag))
        .count();
    adn_core::fasta::write_archive(path, oligos, scheme, manifests)
        .map(|_| ())
        .map_err(|e| format!("impossible de réécrire {} : {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("adn-web-jobs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn a_job_goes_through_its_states() {
        let registry = JobRegistry::new(directory("states"), 2, None).unwrap();
        let job = registry.submit(JobKind::Encode, Some("a.txt".into()));
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.stage.as_deref(), Some("en attente"));

        registry.set_stage(&job.id, "encodage");
        let running = registry.get(&job.id).unwrap();
        assert_eq!(running.status, JobStatus::Running);
        assert_eq!(running.stage.as_deref(), Some("encodage"));
        assert!(!running.is_finished());

        registry.finish(
            &job.id,
            Ok(Outcome {
                file_name: "a.txt.fasta".into(),
                sha256: "ab".repeat(32),
                size: 1234,
                stats: serde_json::json!({}),
            }),
        );
        let done = registry.get(&job.id).unwrap();
        assert_eq!(done.status, JobStatus::Complete);
        assert_eq!(
            done.download_url,
            Some(format!("/download/fasta/{}", job.id))
        );
        assert_eq!(done.file_name.as_deref(), Some("a.txt.fasta"));
        assert!(done.stage.is_none());

        // Une tâche absente ne fait rien lever.
        registry.set_stage("absente", "rien");
        registry.finish("absente", Err("rien".into()));
        assert!(registry.get("absente").is_none());
        let _ = std::fs::remove_dir_all(directory("states"));
    }

    #[test]
    fn a_failed_job_explains_itself_and_drops_its_partial_file() {
        let registry = JobRegistry::new(directory("failed"), 2, None).unwrap();
        let job = registry.submit(JobKind::Decode, Some("a.fasta".into()));
        let partial = registry.result_path(&job.id, JobKind::Decode);
        std::fs::write(&partial, "inachevé").unwrap();

        registry.finish(&job.id, Err("séquences illisibles".into()));
        let failed = registry.get(&job.id).unwrap();
        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("séquences illisibles"));
        assert!(failed.download_url.is_none());
        assert!(!partial.exists());
        let _ = std::fs::remove_dir_all(directory("failed"));
    }

    #[test]
    fn the_newest_jobs_come_first_and_counts_follow() {
        let registry = JobRegistry::new(directory("list"), 2, None).unwrap();
        let first = registry.submit(JobKind::Encode, None);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = registry.submit(JobKind::Decode, None);
        registry.finish(&first.id, Err("échec".into()));

        let listed = registry.list();
        assert_eq!(
            listed.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec![second.id.as_str(), first.id.as_str()]
        );
        assert_eq!(
            registry.counts(),
            JobCounts {
                queued: 1,
                running: 0,
                complete: 0,
                failed: 1
            }
        );
        assert_eq!(registry.get("inconnue"), None);
        let _ = std::fs::remove_dir_all(directory("list"));
    }

    #[test]
    fn removing_a_job_deletes_its_files() {
        let registry = JobRegistry::new(directory("remove"), 2, None).unwrap();
        let job = registry.submit(JobKind::Encode, None);
        let fasta = registry.result_path(&job.id, JobKind::Encode);
        std::fs::write(&fasta, b">adn\nACGT\n").unwrap();
        registry.finish(
            &job.id,
            Ok(Outcome {
                file_name: "a.fasta".into(),
                sha256: String::new(),
                size: 10,
                stats: serde_json::json!({}),
            }),
        );

        assert_eq!(registry.remove(&job.id), Removal::Removed);
        assert!(!fasta.exists());
        assert!(registry.get(&job.id).is_none());
        assert_eq!(registry.remove(&job.id), Removal::Unknown, "déjà retirée");
        let _ = std::fs::remove_dir_all(directory("remove"));
    }

    #[test]
    fn a_running_job_cannot_be_removed() {
        let registry = JobRegistry::new(directory("running"), 1, None).unwrap();
        let queued = registry.submit(JobKind::Encode, None);
        assert_eq!(registry.remove(&queued.id), Removal::Busy);
        assert!(registry.get(&queued.id).is_some(), "elle continue");

        registry.set_stage(&queued.id, "encodage");
        assert_eq!(registry.remove(&queued.id), Removal::Busy);
        assert_eq!(registry.counts().running, 1);

        // Une fois terminée, elle se retire.
        registry.finish(&queued.id, Err("échec".into()));
        assert_eq!(registry.remove(&queued.id), Removal::Removed);
        assert_eq!(registry.remove("inconnue"), Removal::Unknown);
        let _ = std::fs::remove_dir_all(directory("remove"));
    }

    #[test]
    fn finished_jobs_expire_but_not_the_others() {
        let registry = JobRegistry::new(directory("expire"), 2, None).unwrap();
        let now = Timestamp::now();
        let later = now.checked_add(jiff::Span::new().minutes(30)).unwrap();
        assert!(
            registry.expired(now, 0).is_empty(),
            "une durée nulle garde tout"
        );

        let done = registry.submit(JobKind::Encode, None);
        registry.finish(&done.id, Err("échec".into()));
        let running = registry.submit(JobKind::Encode, None);
        registry.set_stage(&running.id, "encodage");

        assert!(
            registry.expired(now, 10).is_empty(),
            "rien n'a eu le temps de vieillir"
        );
        // Une demi-heure plus tard, seule la tâche terminée a expiré.
        assert_eq!(
            registry.expired(later, 10),
            vec![done.id.clone()],
            "la tâche en cours est épargnée"
        );
        assert!(registry.expired(later, 60).is_empty());
        assert!(
            registry.expired(later, 0).is_empty(),
            "une durée nulle garde tout"
        );
        let _ = std::fs::remove_dir_all(directory("expire"));
    }

    #[tokio::test]
    async fn at_most_concurrent_jobs_work_at_once() {
        let registry = Arc::new(JobRegistry::new(directory("slots"), 2, None).unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let ids: Vec<String> = (0..6)
            .map(|_| registry.submit(JobKind::Encode, None).id)
            .collect();
        for id in &ids {
            let (active, peak) = (Arc::clone(&active), Arc::clone(&peak));
            launch(Arc::clone(&registry), id, move |_, _| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(30));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(Outcome {
                    file_name: "a".into(),
                    sha256: String::new(),
                    size: 0,
                    stats: serde_json::json!({}),
                })
            });
        }
        let mut polls = 0;
        while registry.counts().complete < ids.len() {
            assert!(polls < 2000, "les tâches ne se terminent pas");
            polls += 1;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert!(registry
            .list()
            .iter()
            .all(|job| job.status == JobStatus::Complete));
        let _ = std::fs::remove_dir_all(directory("slots"));
    }

    #[tokio::test]
    async fn a_panicking_task_fails_instead_of_hanging() {
        let registry = Arc::new(JobRegistry::new(directory("panic"), 1, None).unwrap());
        let job = registry.submit(JobKind::Encode, None);
        launch(Arc::clone(&registry), &job.id, |_, _| {
            panic!("panique volontaire")
        });
        let mut polls = 0;
        while !registry.get(&job.id).unwrap().is_finished() {
            assert!(polls < 2000);
            polls += 1;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let job = registry.get(&job.id).unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert!(job.error.as_deref().unwrap().contains("interrompue"));
        let _ = std::fs::remove_dir_all(directory("panic"));
    }

    #[tokio::test]
    async fn archives_survive_a_restart_when_the_database_is_enabled() {
        use adn_storage::DatabaseManager;

        let root = directory("restore");
        std::fs::create_dir_all(&root).unwrap();
        let pool = {
            let mut manager = DatabaseManager::new(adn_storage::DatabaseConfig::sqlite(
                root.join("base.db").to_string_lossy().into_owned(),
            ));
            manager.initialize().await.unwrap();
            manager.shared_pool().unwrap()
        };
        let repository = SequenceRepository::new(pool);
        let oligos: Vec<Vec<u8>> = (0..7).map(|_| b"ACGT".repeat(10)).collect();
        let info = ArchiveInfo {
            id: "0b0e0000-0000-4000-8000-000000000001".into(),
            file_name: "lettre.txt".into(),
            sha256: "cd".repeat(32),
            scheme: "fountain".into(),
            stats: "{\"oligos\": 7}".into(),
        };
        repository.save_archive(&info, &oligos).await.unwrap();

        // Un autre serveur démarre : le fichier n'a jamais existé dans son dossier.
        let registry = JobRegistry::new(root.join("uploads"), 1, Some(repository.clone())).unwrap();
        assert_eq!(registry.restore().await.unwrap(), 1);
        let job = registry.get(&info.id).unwrap();
        assert_eq!(job.status, JobStatus::Complete);
        assert_eq!(job.file_name.as_deref(), Some("lettre.txt.fasta"));
        assert_eq!(job.source_name.as_deref(), Some("lettre.txt"));
        let rebuilt = registry.result_path(&info.id, JobKind::Encode);
        let records = adn_core::fasta::read_file(&rebuilt).unwrap();
        assert_eq!(records.len(), 7);
        assert_eq!(records[0].sequence, b"ACGT".repeat(10));

        // Un second démarrage retrouve le fichier, sans le réécrire.
        std::fs::write(&rebuilt, b"exactement tel quel\n").unwrap();
        let registry = JobRegistry::new(root.join("uploads"), 1, Some(repository)).unwrap();
        assert_eq!(registry.restore().await.unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(&rebuilt).unwrap(),
            "exactement tel quel\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_database_is_not_required() {
        let registry = JobRegistry::new(directory("nodb"), 1, None).unwrap();
        assert!(registry.database().is_none());
        assert!(registry
            .result_path("x", JobKind::Encode)
            .ends_with("x.fasta"));
        assert!(registry
            .result_path("x", JobKind::Decode)
            .ends_with("x.bin"));
        let _ = std::fs::remove_dir_all(directory("nodb"));
    }
}
