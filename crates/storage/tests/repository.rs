//! Conservation des oligos sur SQLite, et sur PostgreSQL quand une base de test est fournie.

use adn_storage::{
    ArchiveInfo, DatabaseConfig, DatabaseManager, DatabasePool, Inventory, SequenceRepository,
    StorageError,
};
use std::sync::Arc;

/// Oligos factices, distincts et reconnaissables par leur rang.
fn sequences(count: usize) -> Vec<Vec<u8>> {
    (0..count)
        .map(|rank| format!("ACGT{}", rank).into_bytes())
        .collect()
}

async fn repository(config: DatabaseConfig) -> Arc<DatabasePool> {
    let mut manager = DatabaseManager::new(config);
    manager
        .initialize()
        .await
        .expect("ouvrir la base et appliquer les migrations");
    manager.shared_pool().expect("le pool est prêt")
}

fn repository_on(pool: &Arc<DatabasePool>) -> SequenceRepository {
    SequenceRepository::new(Arc::clone(pool))
}

fn info(id: &str) -> ArchiveInfo {
    ArchiveInfo {
        id: id.to_string(),
        file_name: format!("{id}.txt"),
        sha256: "ab".repeat(32),
        scheme: "fountain".to_string(),
        stats: "{}".to_string(),
    }
}

#[tokio::test]
async fn oligos_come_back_in_order_across_batches() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);

    // Trois lots : 100, 100 et 50 lignes.
    let written = repository
        .save_archive(&info("archive.fa"), &sequences(250))
        .await
        .expect("écrire l'archive");
    assert_eq!(written, 250);

    let read = repository
        .archive_sequences("archive.fa")
        .await
        .expect("lire l'archive");
    assert_eq!(read.len(), 250);
    for (rank, sequence) in read.iter().enumerate() {
        assert_eq!(sequence, &format!("ACGT{rank}").into_bytes());
    }
    assert_eq!(repository.count().await.expect("compter"), 250);
}

#[tokio::test]
async fn archives_are_independent() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);

    repository
        .save_archive(&info("a.fa"), &sequences(3))
        .await
        .expect("écrire");
    repository
        .save_archive(&info("b.fa"), &sequences(1005))
        .await
        .expect("écrire");

    assert_eq!(
        repository
            .archive_sequences("a.fa")
            .await
            .expect("lire")
            .len(),
        3
    );
    let b = repository.archive_sequences("b.fa").await.expect("lire");
    assert_eq!(b.len(), 1005);
    assert_eq!(b[1004], "ACGT1004".as_bytes());
    assert!(repository
        .archive_sequences("absente.fa")
        .await
        .expect("lire")
        .is_empty());

    let inventory = repository.inventory().await.expect("inventaire");
    assert_eq!(inventory.archives, 2);
    assert_eq!(inventory.oligos, 1008);
}

#[tokio::test]
async fn deleting_an_archive_leaves_the_others() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);
    repository
        .save_archive(&info("a.fa"), &sequences(4))
        .await
        .expect("écrire");
    repository
        .save_archive(&info("b.fa"), &sequences(6))
        .await
        .expect("écrire");

    assert_eq!(
        repository.delete_archive("a.fa").await.expect("supprimer"),
        4
    );
    assert_eq!(
        repository.delete_archive("a.fa").await.expect("supprimer"),
        0
    );
    assert_eq!(repository.count().await.expect("compter"), 6);
    assert_eq!(
        repository
            .archive_sequences("b.fa")
            .await
            .expect("lire")
            .len(),
        6
    );
}

#[tokio::test]
async fn an_oligo_is_found_by_its_identifier() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);
    repository
        .save_archive(&info("a.fa"), &sequences(12))
        .await
        .expect("écrire");

    let tenth = repository
        .get(10)
        .await
        .expect("lire")
        .expect("l'oligo existe");
    assert_eq!(tenth.id, 10);
    assert_eq!(tenth.sequence, "ACGT9".as_bytes());
    assert_eq!(tenth.archive, "a.fa");
    assert_eq!(tenth.position, 9);
    assert!(!tenth.uuid.is_nil());

    assert!(repository.get(999).await.expect("lire").is_none());
}

#[tokio::test]
async fn a_refused_sequence_leaves_nothing_behind() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);

    // Les 250 premiers oligos sont valables et tiennent en trois lots ; le dernier ne l'est pas.
    let mut mixed = sequences(250);
    mixed.push(vec![0xC3, 0x28]);
    let error = repository.save_archive(&info("archive.fa"), &mixed).await;
    assert!(
        matches!(error, Err(StorageError::ConfigError(_))),
        "{error:?}"
    );
    assert_eq!(repository.count().await.expect("compter"), 0);
    assert!(repository
        .archive_sequences("archive.fa")
        .await
        .expect("lire")
        .is_empty());

    assert!(repository
        .save_archive(&info("  "), &sequences(2))
        .await
        .is_err());
    assert_eq!(
        repository
            .save_archive(&info("vide.fa"), &[])
            .await
            .expect("rien à écrire"),
        0
    );
    assert_eq!(
        repository.inventory().await.expect("inventaire").archives,
        0
    );

    // Les autres champs sont vérifiés de la même façon.
    let mut incomplete = info("a.fa");
    incomplete.sha256.clear();
    assert!(repository
        .save_archive(&incomplete, &sequences(1))
        .await
        .is_err());

    // Les mesures doivent être un objet JSON ; elles sont rendues telles quelles.
    let mut unmeasured = info("a.fa");
    unmeasured.stats = "{pas du JSON".to_string();
    assert!(repository
        .save_archive(&unmeasured, &sequences(1))
        .await
        .is_err());
    let mut measured = info("a.fa");
    measured.stats = r#"{"oligos": 3}"#.to_string();
    repository
        .save_archive(&measured, &sequences(3))
        .await
        .expect("écrire");
    let read = repository.archive("a.fa").await.expect("lire").unwrap();
    assert_eq!(read.info.stats, r#"{"oligos": 3}"#);
}

#[tokio::test]
async fn archives_are_described_and_listed_newest_first() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);
    repository
        .save_archive(&info("premiere.fa"), &sequences(5))
        .await
        .expect("écrire");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    repository
        .save_archive(&info("seconde.fa"), &sequences(8))
        .await
        .expect("écrire");

    let listed = repository.archives().await.expect("lister");
    assert_eq!(
        listed
            .iter()
            .map(|archive| archive.info.id.as_str())
            .collect::<Vec<_>>(),
        vec!["seconde.fa", "premiere.fa"]
    );

    let first = repository
        .archive("premiere.fa")
        .await
        .expect("lire")
        .expect("l'archive existe");
    assert_eq!(first.info, info("premiere.fa"));
    assert_eq!(first.oligos, 5);
    assert!(first.created_at.contains('T'));
    assert!(repository
        .archive("absente.fa")
        .await
        .expect("lire")
        .is_none());
}

#[tokio::test]
async fn saving_again_under_the_same_name_replaces_the_archive() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);
    repository
        .save_archive(&info("a.fa"), &sequences(150))
        .await
        .expect("écrire");
    repository
        .save_archive(&info("a.fa"), &sequences(3))
        .await
        .expect("écrire");

    let read = repository.archive_sequences("a.fa").await.expect("lire");
    assert_eq!(read.len(), 3);
    assert_eq!(read[2], "ACGT2".as_bytes());
    assert_eq!(
        repository
            .archive("a.fa")
            .await
            .expect("lire")
            .expect("l'archive")
            .oligos,
        3
    );
    assert_eq!(
        repository.inventory().await.expect("inventaire"),
        Inventory {
            archives: 1,
            oligos: 3
        }
    );
}

#[tokio::test]
async fn sequences_are_stored_trimmed() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);
    let padded = vec![b"  ACGT\n".to_vec()];
    repository
        .save_archive(&info("a.fa"), &padded)
        .await
        .expect("écrire");
    assert_eq!(
        repository.archive_sequences("a.fa").await.expect("lire"),
        vec![b"ACGT".to_vec()]
    );
}

#[tokio::test]
async fn rows_from_the_first_version_are_still_listed() {
    let pool = repository(DatabaseConfig::sqlite(":memory:")).await;
    let repository = repository_on(&pool);
    // Une ligne écrite par la version 0.1 : ni archive ni rang.
    pool.execute(
        "INSERT INTO sequences (uuid, sequence_data, metadata, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5)",
        &[
            "11111111-1111-4111-8111-111111111111".into(),
            "ACGTACGT".into(),
            "{}".into(),
            "2024-01-01T00:00:00+00:00".into(),
            "2024-01-01T00:00:00+00:00".into(),
        ],
    )
    .await
    .expect("écrire une ligne ancienne");
    repository
        .save_archive(&info("a.fa"), &sequences(2))
        .await
        .expect("écrire");

    let inventory = repository.inventory().await.expect("inventaire");
    assert_eq!(inventory.archives, 1);
    assert_eq!(inventory.oligos, 3);

    let legacy = repository
        .get(1)
        .await
        .expect("lire")
        .expect("la ligne existe");
    assert_eq!(legacy.archive, "");
    assert_eq!(legacy.position, 0);
}

#[tokio::test]
async fn a_file_database_survives_a_restart_and_repeated_migrations() {
    let directory = std::env::temp_dir().join(format!("adn-storage-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("créer le dossier de test");
    let path = directory.join("base.db");
    let path = path.to_str().expect("chemin en UTF-8").to_string();
    let _ = std::fs::remove_file(&path);

    let pool = repository(DatabaseConfig::sqlite(path.clone())).await;
    repository_on(&pool)
        .save_archive(&info("a.fa"), &sequences(7))
        .await
        .expect("écrire");
    drop(pool);

    // Redémarrage : les migrations déjà appliquées ne sont pas rejouées.
    let pool = repository(DatabaseConfig::sqlite(path.clone())).await;
    let repository = repository_on(&pool);
    assert_eq!(
        repository
            .archive_sequences("a.fa")
            .await
            .expect("lire")
            .len(),
        7
    );

    // Plusieurs connexions sur un fichier.
    let manager = DatabaseManager::new(DatabaseConfig::sqlite(path.clone()));
    assert_eq!(manager.config().max_connections, 5);

    let _ = std::fs::remove_dir_all(&directory);
}

/// Base PostgreSQL de test : `STORAGE_TEST_DATABASE_URL=postgres://…`. Elle doit être vierge, ou ne
/// contenir que les archives de test (`test-archive-adn*`), qui sont supprimées au départ.
#[tokio::test]
#[ignore = "nécessite un serveur PostgreSQL (STORAGE_TEST_DATABASE_URL)"]
async fn postgres_stores_the_same_way() {
    let url = std::env::var("STORAGE_TEST_DATABASE_URL").expect("STORAGE_TEST_DATABASE_URL");
    let pool = repository(DatabaseConfig::postgres(url)).await;
    let repository = repository_on(&pool);
    const NAME: &str = "test-archive-adn";
    const OTHER: &str = "test-archive-adn-autre";
    repository.delete_archive(NAME).await.expect("nettoyer");
    repository.delete_archive(OTHER).await.expect("nettoyer");
    let before = repository.inventory().await.expect("inventaire");

    // Trois lots : 100, 100 et 50 lignes.
    repository
        .save_archive(&info(NAME), &sequences(250))
        .await
        .expect("écrire");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    repository
        .save_archive(&info(OTHER), &sequences(4))
        .await
        .expect("écrire");
    let read = repository.archive_sequences(NAME).await.expect("lire");
    assert_eq!(read.len(), 250);
    assert_eq!(read[0], "ACGT0".as_bytes());
    assert_eq!(read[249], "ACGT249".as_bytes());

    // Description, liste (plus récente d'abord), inventaire.
    let archive = repository
        .archive(NAME)
        .await
        .expect("lire")
        .expect("l'archive existe");
    assert_eq!(archive.info, info(NAME));
    assert_eq!(archive.oligos, 250);
    let listed = repository.archives().await.expect("lister");
    assert_eq!(listed[0].info.id, OTHER);
    assert_eq!(listed[1].info.id, NAME);
    let after = repository.inventory().await.expect("inventaire");
    assert_eq!(after.archives, before.archives + 2);
    assert_eq!(after.oligos, before.oligos + 254);

    // La clé primaire n'a pas la même valeur d'un serveur à l'autre : on cherche par archive.
    let ids = repository.count().await.expect("compter");
    assert!(ids >= 254);

    // Un oligo et son rang.
    let position = listed[0].clone();
    assert_eq!(position.oligos, 4);

    // Remplacement sous le même nom.
    repository
        .save_archive(&info(NAME), &sequences(3))
        .await
        .expect("écrire");
    assert_eq!(
        repository
            .archive_sequences(NAME)
            .await
            .expect("lire")
            .len(),
        3
    );

    assert_eq!(repository.delete_archive(NAME).await.expect("supprimer"), 3);
    assert_eq!(
        repository.delete_archive(OTHER).await.expect("supprimer"),
        4
    );
    assert!(repository.archive(NAME).await.expect("lire").is_none());
    assert!(repository
        .archive_sequences(OTHER)
        .await
        .expect("lire")
        .is_empty());
}
