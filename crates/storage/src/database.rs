//! Connexion aux bases de données : SQLite ou PostgreSQL.
//!
//! Les deux bases sont pilotées par le même code, avec les mêmes requêtes à paramètres
//! numérotés (`$1`, `$2`…), que SQLite accepte comme PostgreSQL.

use sqlx::postgres::{PgPoolOptions, Postgres};
use sqlx::sqlite::{Sqlite, SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Pool;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

/// Base de données utilisée.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    Sqlite,
    Postgres,
}

/// Paramètres de connexion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub db_type: DatabaseType,
    /// Chemin d'un fichier SQLite, `:memory:`, ou URL PostgreSQL.
    pub connection_string: String,
    /// Nombre maximal de connexions ouvertes.
    pub max_connections: u32,
}

impl DatabaseConfig {
    /// Base SQLite dans un fichier.
    pub fn sqlite(path: impl Into<String>) -> Self {
        Self {
            db_type: DatabaseType::Sqlite,
            connection_string: path.into(),
            max_connections: 5,
        }
    }

    /// Base PostgreSQL, par URL (`postgres://utilisateur:mot_de_passe@hote/base`).
    pub fn postgres(url: impl Into<String>) -> Self {
        Self {
            db_type: DatabaseType::Postgres,
            connection_string: url.into(),
            max_connections: 5,
        }
    }

    /// Vérifie la cohérence des paramètres.
    pub fn validate(&self) -> crate::Result<()> {
        if self.connection_string.trim().is_empty() {
            return Err(crate::StorageError::ConfigError(
                "chaîne de connexion vide".to_string(),
            ));
        }
        if self.max_connections == 0 {
            return Err(crate::StorageError::ConfigError(
                "au moins une connexion est nécessaire".to_string(),
            ));
        }
        Ok(())
    }
}

/// Gestionnaire de la connexion : il possède le pool.
pub struct DatabaseManager {
    config: DatabaseConfig,
    pool: Option<Arc<DatabasePool>>,
}

/// Adresse comprise par sqlx pour une base SQLite. sqlx exige une URL `sqlite://`.
fn sqlite_url(connection: &str) -> String {
    if connection.starts_with("sqlite:") {
        connection.to_string()
    } else if is_memory_database(connection) {
        "sqlite::memory:".to_string()
    } else {
        format!("sqlite://{connection}")
    }
}

/// Vrai pour les deux notations d'une base en mémoire.
fn is_memory_database(connection: &str) -> bool {
    matches!(connection, ":memory:" | "sqlite::memory:")
}

impl DatabaseManager {
    /// Prépare un gestionnaire, sans se connecter.
    pub fn new(config: DatabaseConfig) -> Self {
        Self { config, pool: None }
    }

    /// Paramètres du gestionnaire.
    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// Une part du pool, à donner aux dépôts et aux gestionnaires de requêtes.
    pub fn shared_pool(&self) -> crate::Result<Arc<DatabasePool>> {
        Ok(Arc::clone(self.pool()?))
    }

    /// Se connecte.
    pub async fn connect(&mut self) -> crate::Result<()> {
        self.config.validate()?;
        let pool = match self.config.db_type {
            DatabaseType::Sqlite => DatabasePool::Sqlite({
                let connection = self.config.connection_string.as_str();
                let is_memory = is_memory_database(connection);
                let options = SqliteConnectOptions::from_str(&sqlite_url(connection))?
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);
                // Une base en mémoire est privée à chaque connexion : le pool est limité à
                // une, pour que toutes les requêtes voient les mêmes données. sqlx ne crée
                // pas le fichier sans `create_if_missing`.
                SqlitePoolOptions::new()
                    .max_connections(if is_memory {
                        1
                    } else {
                        self.config.max_connections
                    })
                    .acquire_timeout(Duration::from_secs(10))
                    .connect_with(options)
                    .await?
            }),
            DatabaseType::Postgres => DatabasePool::Postgres(
                PgPoolOptions::new()
                    .max_connections(self.config.max_connections)
                    .acquire_timeout(Duration::from_secs(10))
                    .connect(&self.config.connection_string)
                    .await?,
            ),
        };
        self.pool = Some(Arc::new(pool));
        info!("Connexion établie");
        Ok(())
    }

    /// Se connecte et applique les migrations.
    pub async fn initialize(&mut self) -> crate::Result<()> {
        self.connect().await?;
        self.migrate().await
    }

    /// Applique les migrations.
    pub async fn migrate(&self) -> crate::Result<()> {
        match self.pool()?.as_ref() {
            DatabasePool::Sqlite(pool) => {
                sqlx::migrate!("./migrations/sqlite").run(pool).await?;
            }
            DatabasePool::Postgres(pool) => {
                sqlx::migrate!("./migrations/postgres").run(pool).await?;
            }
        }
        info!("Migrations appliquées");
        Ok(())
    }

    /// Le pool, si la connexion est établie.
    pub fn pool(&self) -> crate::Result<&Arc<DatabasePool>> {
        self.pool.as_ref().ok_or_else(|| {
            crate::StorageError::ConnectionError("base de données non connectée".to_string())
        })
    }

    /// Vérifie que la base répond.
    pub async fn health_check(&self) -> crate::Result<()> {
        self.pool()?.execute("SELECT 1", &[]).await?;
        Ok(())
    }
}

/// Pool de connexions, pour l'un ou l'autre des deux bases.
#[derive(Clone)]
pub enum DatabasePool {
    Sqlite(Pool<Sqlite>),
    Postgres(Pool<Postgres>),
}

/// Exécute le même code sur l'un ou l'autre des pools. `executor` désigne, dans `body`, une
/// référence au pool (`&Pool<Sqlite>` ou `&Pool<Postgres>`) : `body` est compilé deux fois.
macro_rules! on_pool {
    ($pool:expr, $executor:ident => $body:expr) => {
        match $pool {
            DatabasePool::Sqlite(pool) => {
                let $executor = pool;
                $body
            }
            DatabasePool::Postgres(pool) => {
                let $executor = pool;
                $body
            }
        }
    };
}

pub(crate) use on_pool;

/// Une valeur passée à une requête. Dates et identifiants universels sont au format texte, lu
/// pareil par les deux bases ; les nombres sont des entiers 64 bits, car PostgreSQL refuse de
/// comparer un `bigint` à un texte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Argument {
    Text(String),
    Int(i64),
}

impl From<String> for Argument {
    fn from(text: String) -> Self {
        Argument::Text(text)
    }
}

impl From<&str> for Argument {
    fn from(text: &str) -> Self {
        Argument::Text(text.to_string())
    }
}

impl From<i64> for Argument {
    fn from(value: i64) -> Self {
        Argument::Int(value)
    }
}

/// Lie les arguments à la requête.
fn bind<'q, 'e, DB: sqlx::Database>(
    sql: impl sqlx::SqlSafeStr,
    arguments: &[Argument],
) -> sqlx::query::Query<'q, DB, <DB as sqlx::Database>::Arguments>
where
    String: sqlx::Encode<'e, DB> + sqlx::Type<DB>,
    i64: sqlx::Encode<'e, DB> + sqlx::Type<DB>,
{
    let mut query = sqlx::query(sql);
    for argument in arguments {
        query = match argument {
            Argument::Text(text) => query.bind(text.clone()),
            Argument::Int(value) => query.bind(*value),
        };
    }
    query
}

/// Nombre de lignes touchées, lu pareil sur les deux bases.
pub(crate) trait RowsAffected {
    fn affected(&self) -> u64;
}

impl RowsAffected for sqlx::sqlite::SqliteQueryResult {
    fn affected(&self) -> u64 {
        self.rows_affected()
    }
}

impl RowsAffected for sqlx::postgres::PgQueryResult {
    fn affected(&self) -> u64 {
        self.rows_affected()
    }
}

/// Exécute une requête qui ne rend rien ; retourne le nombre de lignes touchées.
pub(crate) async fn execute<'a, DB>(
    executor: impl sqlx::Executor<'a, Database = DB>,
    sql: impl sqlx::SqlSafeStr,
    arguments: &[Argument],
) -> crate::Result<u64>
where
    DB: sqlx::Database,
    String: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
    i64: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
    <DB as sqlx::Database>::Arguments: sqlx::IntoArguments<DB>,
    <DB as sqlx::Database>::QueryResult: RowsAffected,
{
    Ok(bind::<DB>(sql, arguments)
        .execute(executor)
        .await?
        .affected())
}

/// Lit des lignes et les convertit.
///
/// `convert` reçoit chaque ligne par le trait `sqlx::Row` : les colonnes se lisent par leur nom.
pub(crate) async fn fetch<'a, DB, T>(
    executor: impl sqlx::Executor<'a, Database = DB>,
    sql: impl sqlx::SqlSafeStr,
    arguments: &[Argument],
    convert: impl for<'r> Fn(&'r <DB as sqlx::Database>::Row) -> crate::Result<T>,
) -> crate::Result<Vec<T>>
where
    DB: sqlx::Database,
    String: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
    i64: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
    <DB as sqlx::Database>::Arguments: sqlx::IntoArguments<DB>,
{
    let rows: Vec<<DB as sqlx::Database>::Row> =
        bind::<DB>(sql, arguments).fetch_all(executor).await?;
    rows.iter().map(convert).collect()
}

impl DatabasePool {
    /// Exécute une requête autonome ; retourne le nombre de lignes touchées. Une requête
    /// construite à la main passe par [`sqlx::AssertSqlSafe`].
    pub async fn execute(
        &self,
        sql: impl sqlx::SqlSafeStr,
        arguments: &[Argument],
    ) -> crate::Result<u64> {
        on_pool!(self, executor => execute(executor, sql, arguments).await)
    }

    /// Ouvre une transaction.
    pub async fn begin(&self) -> crate::Result<Transaction<'_>> {
        Ok(match self {
            DatabasePool::Sqlite(pool) => Transaction::Sqlite(pool.begin().await?),
            DatabasePool::Postgres(pool) => Transaction::Postgres(pool.begin().await?),
        })
    }
}

/// Transaction en cours.
pub enum Transaction<'a> {
    Sqlite(sqlx::Transaction<'a, Sqlite>),
    Postgres(sqlx::Transaction<'a, Postgres>),
}

impl Transaction<'_> {
    /// Exécute une requête dans la transaction.
    pub async fn execute(
        &mut self,
        sql: impl sqlx::SqlSafeStr,
        arguments: &[Argument],
    ) -> crate::Result<u64> {
        match self {
            Transaction::Sqlite(transaction) => execute(&mut **transaction, sql, arguments).await,
            Transaction::Postgres(transaction) => execute(&mut **transaction, sql, arguments).await,
        }
    }

    /// Valide la transaction. Un abandon implicite (drop) annule ce qui a été fait.
    pub async fn commit(self) -> crate::Result<()> {
        match self {
            Transaction::Sqlite(transaction) => transaction.commit().await?,
            Transaction::Postgres(transaction) => transaction.commit().await?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_urls() {
        assert_eq!(sqlite_url(":memory:"), "sqlite::memory:");
        assert_eq!(sqlite_url("sqlite::memory:"), "sqlite::memory:");
        assert_eq!(sqlite_url("base.db"), "sqlite://base.db");
        assert_eq!(sqlite_url("sqlite://base.db"), "sqlite://base.db");
        assert_eq!(sqlite_url("data/base.db"), "sqlite://data/base.db");
    }

    #[test]
    fn configuration_is_checked() {
        assert!(DatabaseConfig::sqlite("base.db").validate().is_ok());
        assert!(DatabaseConfig::postgres("postgres://h/base")
            .validate()
            .is_ok());
        assert!(DatabaseConfig::sqlite("  ").validate().is_err());
        let mut config = DatabaseConfig::sqlite("base.db");
        config.max_connections = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_manager_without_connection_reports_it() {
        let manager = DatabaseManager::new(DatabaseConfig::sqlite(":memory:"));
        assert!(manager.pool().is_err());
        assert!(manager.shared_pool().is_err());
    }
}
