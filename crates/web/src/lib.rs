//! `adn-web` : bibliothèque du serveur web qui archive des fichiers en séquences d'ADN et les
//! restitue. [`serve`] démarre le serveur ; la macro [`app!`] construit l'application,
//! pour le serveur comme pour les tests.

pub mod config;
pub mod jobs;
pub mod pipeline;
pub mod routes;

use actix_web::{web, HttpServer};
use adn_storage::{DatabaseManager, SequenceRepository};
use config::AppConfig;
use jiff::Timestamp;
use jobs::JobRegistry;
use routes::{AppState, Limits};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, Span};
use tracing_actix_web::{DefaultRootSpanBuilder, RootSpanBuilder, TracingLogger};

/// Gabarits des pages, tous dans le dossier indiqué par la configuration.
const TEMPLATES: [&str; 4] = ["base.html", "index.html", "encode.html", "decode.html"];

/// Démarre le serveur. Rend une erreur si la configuration, la base de données, les gabarits
/// ou l'écoute ne conviennent pas.
pub async fn serve(mut config: AppConfig) -> Result<(), String> {
    init_logging(&config);

    let pool = if config.database.enabled {
        let mut manager = DatabaseManager::new(config.database.connection());
        manager
            .initialize()
            .await
            .map_err(|error| format!("base de données indisponible : {error}"))?;
        Some(manager.shared_pool().map_err(|error| error.to_string())?)
    } else {
        None
    };
    let tera = load_templates(&config.server.templates)?;
    let registry = Arc::new(
        JobRegistry::new(
            &config.server.storage_dir,
            config.server.max_concurrent_jobs,
            pool.map(SequenceRepository::new),
        )
        .map_err(|error| {
            format!(
                "impossible de préparer le dossier des résultats {} : {error}",
                config.server.storage_dir.display()
            )
        })?,
    );
    let found = registry.restore().await?;
    if found > 0 {
        info!("{found} archive(s) retrouvée(s) dans la base de données");
    }
    sweep_expired(Arc::clone(&registry), config.server.job_retention_minutes);

    if !config.server.static_files.is_dir() {
        return Err(format!(
            "dossier des fichiers statiques introuvable : {}",
            config.server.static_files.display()
        ));
    }
    let state = web::Data::new(AppState {
        jobs: registry,
        tera,
        limits: Limits {
            upload_bytes: config.server.upload_limit,
            concurrent_jobs: config.server.max_concurrent_jobs,
            retention_minutes: config.server.job_retention_minutes,
        },
        started: Timestamp::now(),
    });
    let static_files = config.server.static_files.clone();
    // Les hôtes que les requêtes peuvent désigner : un site tiers ne peut pas, en faisant
    // rebinder son nom de domaine vers `127.0.0.1`, lire les résultats comme s'il était la page
    // d'origine. Un proxy inverse qui préserve l'en-tête `Host` fait déclarer son nom public.
    let policy = routes::HostPolicy::new(
        &config.server.host,
        std::mem::take(&mut config.server.allowed_hosts),
    );

    info!(
        "Serveur ADN {} sur http://{}:{} (fichiers jusqu'à {}, {} tâche(s) à la fois)",
        env!("CARGO_PKG_VERSION"),
        config.server.host,
        config.server.port,
        routes::human_bytes(config.server.upload_limit as u64),
        config.server.max_concurrent_jobs,
    );
    let server = HttpServer::new(move || {
        // Le journal d'accès est le plus extérieur : il note aussi les requêtes refusées.
        app!(state.clone(), &static_files, policy.clone()).wrap(TracingLogger::<AccessLog>::new())
    })
    .workers(config.server.workers)
    .bind((config.server.host.clone(), config.server.port))
    .map_err(|error| {
        format!(
            "écoute impossible sur {}:{} : {error}",
            config.server.host, config.server.port
        )
    })?;
    server
        .run()
        .await
        .map_err(|error| format!("serveur arrêté : {error}"))
}

/// Journal d'accès. `/health` est interrogé toutes les trente secondes par un conteneur : ses
/// visites restent au niveau trace pour ne pas noyer le journal.
#[derive(Debug)]
struct AccessLog;

impl RootSpanBuilder for AccessLog {
    fn on_request_start(request: &actix_web::dev::ServiceRequest) -> Span {
        if request.path() == "/health" {
            tracing::trace_span!("health")
        } else {
            DefaultRootSpanBuilder::on_request_start(request)
        }
    }

    fn on_request_end<B: actix_web::body::MessageBody>(
        span: Span,
        outcome: &Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>,
    ) {
        DefaultRootSpanBuilder::on_request_end(span, outcome);
    }
}

/// Lit les gabarits. Ils sont nommés explicitement : un gabarit manquant est signalé au
/// démarrage, et aucun fichier du dossier n'est lu à l'insu de cette liste.
pub fn load_templates(directory: &Path) -> Result<tera::Tera, String> {
    let mut tera = tera::Tera::default();
    for name in TEMPLATES {
        let path = directory.join(name);
        tera.add_template_file(&path, Some(name))
            .map_err(|error| format!("gabarit {} illisible : {error}", path.display()))?;
    }
    Ok(tera)
}

/// Vrai si l'adresse écoutée ne quitte pas la machine.
pub fn is_loopback(host: &str) -> bool {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0:0:0:0:0:0:0:1"
    ) || host.starts_with("127.")
}

/// Efface, une fois par minute, les résultats plus vieux que la durée de conservation.
fn sweep_expired(registry: Arc<JobRegistry>, retention_minutes: u64) {
    if retention_minutes == 0 {
        info!("Les résultats sont conservés jusqu'à leur suppression");
        return;
    }
    tokio::spawn(async move {
        loop {
            actix_web::rt::time::sleep(Duration::from_secs(60)).await;
            for id in registry.expired(Timestamp::now(), retention_minutes) {
                // Les tâches expirées sont terminées : le retrait réussit toujours.
                if registry.remove(&id) == jobs::Removal::Removed {
                    if let Err(message) = registry.forget_archive(&id).await {
                        error!(
                            "Résultats de {id} effacés, mais pas leur archive en base : {message}"
                        );
                    }
                }
            }
        }
    });
}

/// Journalisation sur la sortie d'erreur : `RUST_LOG` l'emporte sur `logging.level`.
fn init_logging(config: &AppConfig) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(config.log_filter()));
    let logger = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);
    if config.logging.format.eq_ignore_ascii_case("json") {
        logger.json().init();
    } else {
        logger.compact().init();
    }
}

/// Résout un chemin de fichier de configuration donné en argument.
pub fn config_path(argument: Option<std::ffi::OsString>) -> PathBuf {
    argument.map_or_else(|| PathBuf::from("config.toml"), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_addresses_are_recognised() {
        for local in [
            "127.0.0.1",
            "localhost",
            "LOCALHOST",
            "::1",
            "[::1]",
            "127.5.6.7",
            " 127.0.0.1 ",
        ] {
            assert!(is_loopback(local), "{local}");
        }
        for other in ["0.0.0.0", "192.168.1.2", "example.com", "adn.local", ""] {
            assert!(!is_loopback(other), "{other}");
        }
    }

    #[test]
    fn the_templates_ship_with_the_server() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let tera = load_templates(&directory).expect("les gabarits sont présents et valides");
        for name in TEMPLATES {
            assert!(
                tera.get_template_names().any(|loaded| loaded == name),
                "{name}"
            );
        }
        assert!(load_templates(&directory.join("absent")).is_err());
    }

    #[test]
    fn the_configuration_path_defaults_to_config_toml() {
        assert_eq!(config_path(None), PathBuf::from("config.toml"));
        assert_eq!(
            config_path(Some("/etc/adn.toml".into())),
            PathBuf::from("/etc/adn.toml")
        );
    }
}
