//! ADN Storage Web Server
//!
//! Serveur web pour l'encodage/décodage de fichiers en ADN virtuel

use actix_files::Files;
use actix_web::{web, App, HttpResponse, HttpServer, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tera::{Context, Tera};
use tokio::sync::RwLock;

// mod routes;
// mod models;

/// État global de l'application
struct AppState {
    tera: Tera,
    jobs: RwLock<HashMap<String, JobState>>,
}

/// État d'un job d'encodage/décodage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobState {
    id: String,
    status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<JobResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum JobStatus {
    Pending,
    Processing,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JobResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    download_url: Option<String>,
    stats: Option<EncodingStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncodingStats {
    sequence_count: usize,
    avg_length: f64,
    gc_ratio: f64,
    bits_per_base: f64,
    file_size: usize,
    encoded_size: usize,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialiser le logger
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    // Initialiser Tera
    let tera = Tera::new("templates/**/*").expect("Erreur lors de l'initialisation de Tera");

    log::info!("🧬 Démarrage du serveur ADN Storage sur http://127.0.0.1:8080");

    HttpServer::new(move || {
        let app_data = web::Data::new(AppState {
            tera: tera.clone(),
            jobs: RwLock::new(HashMap::new()),
        });

        App::new()
            .app_data(app_data)
            .service(index)
            .service(encode_page)
            .service(decode_page)
            .service(Files::new("/static", "./static").show_files_listing())
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

/// Page d'accueil
#[actix_web::get("/")]
async fn index(data: web::Data<AppState>) -> Result<HttpResponse> {
    let mut ctx = Context::new();
    ctx.insert("title", "ADN Data Storage");

    let rendered = data.tera.render("index.html", &ctx)
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;

    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

/// Page d'encodage
#[actix_web::get("/encode")]
async fn encode_page(data: web::Data<AppState>) -> Result<HttpResponse> {
    let mut ctx = Context::new();
    ctx.insert("title", "Encoder en ADN");

    let rendered = data.tera.render("encode.html", &ctx)
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;

    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}

/// Page de décodage
#[actix_web::get("/decode")]
async fn decode_page(data: web::Data<AppState>) -> Result<HttpResponse> {
    let mut ctx = Context::new();
    ctx.insert("title", "Décoder depuis ADN");

    let rendered = data.tera.render("decode.html", &ctx)
        .map_err(|e| actix_web::error::ErrorInternalServerError(e))?;

    Ok(HttpResponse::Ok().content_type("text/html").body(rendered))
}
