//! Système de logging professionnel pour ADN Core

use tracing_subscriber::{fmt, EnvFilter};

/// Initialise le système de logging
pub fn init_logging() {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_thread_ids(true)
        .with_thread_names(true)
        .init();
}

/// Macro pour le logging des opérations critiques
#[macro_export]
macro_rules! log_operation {
    ($name:expr, $block:block) => {{
        let span = tracing::span!(tracing::Level::INFO, $name);
        let _enter = span.enter();
        tracing::info!("Début de l'opération: {}", $name);
        let result = $block;
        tracing::info!("Fin de l'opération: {}", $name);
        result
    }};
}

/// Macro pour le logging des erreurs
#[macro_export]
macro_rules! log_error {
    ($error:expr) => {{
        tracing::error!("Erreur: {}", $error);
        $error
    }};
}