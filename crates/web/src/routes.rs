//! Pages et API du serveur.
//!
//! L'interface et l'API sont servies par la même origine : il n'y a pas de CORS, et aucune
//! autre page que celle-ci ne peut donc lire les résultats depuis un navigateur.

use actix_files::NamedFile;
use actix_multipart::Multipart;
use actix_web::http::{header, StatusCode};
use actix_web::{delete, get, post, web, HttpResponse, Responder};
use actix_web::{CustomizeResponder, Either};
use adn_core::{Compression, EncodeOptions};
use futures::StreamExt;
use jiff::Timestamp;
use serde::Serialize;
use serde_json::json;
use std::fmt::Write as _;
use std::sync::Arc;
use tracing::{error, warn};
use uuid::Uuid;

use crate::jobs::{self, JobKind, JobRegistry, JobStatus, Removal};
use crate::pipeline::{self, DecodeRequest, EncodeRequest, UNNAMED_FILE};

/// Longueur maximale d'un nom de fichier proposé.
const FILE_NAME_LIMIT: usize = 120;

/// Taille maximale d'un champ texte du formulaire (schéma, compression…).
const FIELD_LIMIT: usize = 16 * 1024;

/// Tâches en attente ou en cours admises, par emplacement de travail. Au-delà, le serveur
/// refuse la requête plutôt que d'accumuler des fichiers en mémoire.
const QUEUE_PER_WORKER: usize = 8;

/// Limites appliquées par le serveur.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Taille maximale des fichiers acceptés, en octets.
    pub upload_bytes: usize,
    /// Tâches menées de front.
    pub concurrent_jobs: usize,
    /// Durée de conservation des résultats, en minutes ; `0` pour ne jamais les effacer.
    pub retention_minutes: u64,
}

/// État partagé par tous les gestionnaires de requêtes.
pub struct AppState {
    pub jobs: Arc<JobRegistry>,
    pub tera: tera::Tera,
    pub limits: Limits,
    pub started: Timestamp,
}

/// Réponse à une création de tâche.
#[derive(Serialize)]
struct Accepted {
    job_id: String,
    status: JobStatus,
    /// Où suivre la tâche.
    location: String,
}

// Pages
// -----

#[get("/")]
pub async fn index(app: web::Data<AppState>) -> HttpResponse {
    page(&app, "index.html", "index", "ADN Data Storage")
}

#[get("/encode")]
pub async fn encode_page(app: web::Data<AppState>) -> HttpResponse {
    page(&app, "encode.html", "encode", "Encoder un fichier")
}

#[get("/decode")]
pub async fn decode_page(app: web::Data<AppState>) -> HttpResponse {
    page(&app, "decode.html", "decode", "Décoder une archive")
}

fn page(app: &AppState, template: &str, name: &str, title: &str) -> HttpResponse {
    let limit = app.limits.upload_bytes;
    let retention = app.limits.retention_minutes;
    let retention_label = if retention == 0 {
        "jusqu'à leur suppression".to_string()
    } else {
        // Une durée ronde s'écrit en heures, le reste en minutes.
        let (value, unit) = if retention.is_multiple_of(60) {
            (retention / 60, "heures")
        } else {
            (retention, "minutes")
        };
        format!("pendant {value} {unit}")
    };
    let database_enabled = app.jobs.database().is_some();
    let redundancy_default = pipeline::default_redundancy();
    let mut context = tera::Context::new();
    context.insert("title", title);
    context.insert("page", name);
    context.insert("version", env!("CARGO_PKG_VERSION"));
    context.insert("upload_limit_bytes", &limit);
    context.insert("upload_limit_label", &human_bytes(limit as u64));
    context.insert("retention_label", &retention_label);
    context.insert("database_enabled", &database_enabled);
    context.insert("redundancy_default", &redundancy_default);
    context.insert("redundancy_min", &pipeline::MIN_REDUNDANCY);
    context.insert("redundancy_max", &pipeline::MAX_REDUNDANCY);
    match app.tera.render(template, &context) {
        Ok(html) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html),
        Err(error) => {
            error!("Impossible de rendre {template} : {error}");
            HttpResponse::InternalServerError().json(json!({ "error": "page indisponible" }))
        }
    }
}

// Encodage
// --------

#[post("/api/encode")]
pub async fn api_encode(app: web::Data<AppState>, mut payload: Multipart) -> HttpResponse {
    let upload = match read_upload(&mut payload, app.limits.upload_bytes).await {
        Ok(upload) => upload,
        Err(refused) => return refused,
    };
    if upload.file_name.is_empty() {
        return refusal(
            StatusCode::BAD_REQUEST,
            "aucun fichier reçu : envoyer un champ « file »",
        );
    }
    let options = match encode_options(&upload.fields) {
        Ok(options) => options,
        Err(refused) => return refused,
    };
    if let Some(refused) = queue_full(&app) {
        return refused;
    }

    let source_name = upload.file_name.clone();
    let job = app.jobs.submit(JobKind::Encode, Some(source_name.clone()));
    let request = EncodeRequest {
        source_name,
        data: upload.data,
        options,
    };
    jobs::launch(Arc::clone(&app.jobs), &job.id, move |registry, id| {
        pipeline::encode(registry, id, request)
    });
    accepted(&job.id)
}

// Décodage
// --------

#[post("/api/decode")]
pub async fn api_decode(app: web::Data<AppState>, mut payload: Multipart) -> HttpResponse {
    let upload = match read_upload(&mut payload, app.limits.upload_bytes).await {
        Ok(upload) => upload,
        Err(refused) => return refused,
    };
    if upload.file_name.is_empty() {
        return refusal(
            StatusCode::BAD_REQUEST,
            "aucune archive reçue : envoyer un champ « file »",
        );
    }
    let mut expected_sha256 = None;
    for (name, value) in &upload.fields {
        if name.trim() != "expected_sha256" {
            return refusal(
                StatusCode::BAD_REQUEST,
                format!("champ inconnu : « {} »", name.trim()),
            );
        }
        match adn_core::checksum::parse_sha256(value.trim()) {
            Some(digest) => expected_sha256 = Some(digest),
            None => {
                return refusal(
                    StatusCode::BAD_REQUEST,
                    "l'empreinte attendue doit compter 64 chiffres hexadécimaux",
                )
            }
        }
    }
    if let Some(refused) = queue_full(&app) {
        return refused;
    }

    let source_name = upload.file_name.clone();
    let job = app.jobs.submit(JobKind::Decode, Some(source_name.clone()));
    let request = DecodeRequest {
        source_name,
        data: upload.data,
        expected_sha256,
    };
    jobs::launch(Arc::clone(&app.jobs), &job.id, move |registry, id| {
        pipeline::decode(registry, id, request)
    });
    accepted(&job.id)
}

// Suivi et résultats
// -------------------

#[get("/api/jobs")]
pub async fn job_list(app: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "jobs": app.jobs.list() }))
}

#[get("/api/jobs/{id}")]
pub async fn job_status(app: web::Data<AppState>, id: web::Path<String>) -> HttpResponse {
    let id = match job_id(&id) {
        Ok(id) => id,
        Err(refused) => return refused,
    };
    match app.jobs.get(&id) {
        Some(job) => HttpResponse::Ok().json(job),
        None => refusal(StatusCode::NOT_FOUND, "tâche inconnue"),
    }
}

#[delete("/api/jobs/{id}")]
pub async fn job_delete(app: web::Data<AppState>, id: web::Path<String>) -> HttpResponse {
    let id = match job_id(&id) {
        Ok(id) => id,
        Err(refused) => return refused,
    };
    match app.jobs.remove(&id) {
        Removal::Unknown => return refusal(StatusCode::NOT_FOUND, "tâche inconnue"),
        // Rien n'annule une tâche : retirée, elle continuerait et laisserait un résultat orphelin.
        Removal::Busy => {
            return refusal(
                StatusCode::CONFLICT,
                "cette tâche est en cours : la retirer une fois terminée",
            )
        }
        Removal::Removed => {}
    }
    if let Err(message) = app.jobs.forget_archive(&id).await {
        error!("Archive {id} effacée du disque mais pas de la base : {message}");
        return refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            "résultats effacés du disque, mais pas de la base de données",
        );
    }
    HttpResponse::NoContent().finish()
}

#[get("/download/fasta/{id}")]
pub async fn download_fasta(app: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    serve_file(&app, &id, JobKind::Encode)
}

#[get("/download/{id}")]
pub async fn download_file(app: web::Data<AppState>, id: web::Path<String>) -> impl Responder {
    serve_file(&app, &id, JobKind::Decode)
}

/// Ne rend un fichier que pour une tâche réussie : ni les résultats inachevés, ni un fichier
/// étranger au dossier des résultats, ne peuvent être demandés. Le fichier part en flux.
fn serve_file(
    app: &AppState,
    raw_id: &str,
    kind: JobKind,
) -> Either<HttpResponse, CustomizeResponder<NamedFile>> {
    let (id, file_name) = match download_target(app, raw_id, kind) {
        Ok(target) => target,
        Err(refused) => return Either::Left(refused),
    };
    let content_type = match kind {
        JobKind::Encode => "text/plain; charset=utf-8",
        JobKind::Decode => "application/octet-stream",
    };
    let path = app.jobs.result_path(&id, kind);
    match NamedFile::open(&path) {
        Ok(file) => Either::Right(
            file.customize()
                .insert_header((header::CONTENT_DISPOSITION, attachment(&file_name)))
                .insert_header((header::CONTENT_TYPE, content_type)),
        ),
        Err(error) => {
            error!("Résultat de {id} introuvable ({}), {error}", path.display());
            Either::Left(refusal(
                StatusCode::NOT_FOUND,
                "le fichier de cette tâche a disparu",
            ))
        }
    }
}

/// Le nom sous lequel proposer le résultat d'une tâche réussie.
// Une réponse d'erreur ne voyage qu'une fois, à l'échec : la boxer ne gagnerait rien.
#[allow(clippy::result_large_err)]
fn download_target(
    app: &AppState,
    raw_id: &str,
    kind: JobKind,
) -> Result<(String, String), HttpResponse> {
    let id = job_id(raw_id)?;
    match app.jobs.get(&id) {
        Some(job) if job.kind == kind => {}
        _ => return Err(refusal(StatusCode::NOT_FOUND, "tâche inconnue")),
    }
    if app
        .jobs
        .get(&id)
        .is_some_and(|job| job.status != JobStatus::Complete)
    {
        return Err(refusal(
            StatusCode::NOT_FOUND,
            "cette tâche n'a rien à télécharger",
        ));
    }
    let file_name = app
        .jobs
        .get(&id)
        .and_then(|job| job.file_name)
        .unwrap_or_else(|| UNNAMED_FILE.to_string());
    Ok((id, file_name))
}

// État du serveur
// ----------------

#[get("/health")]
pub async fn health(app: web::Data<AppState>) -> HttpResponse {
    let mut database = json!({ "enabled": false });
    let mut healthy = true;
    if let Some(repository) = app.jobs.database() {
        match repository.count().await {
            Ok(_) => database = json!({ "enabled": true, "state": "ok" }),
            Err(error) => {
                database =
                    json!({ "enabled": true, "state": "erreur", "error": error.to_string() });
                healthy = false;
            }
        }
    }
    let status = if healthy {
        actix_web::http::StatusCode::OK
    } else {
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    };
    HttpResponse::build(status).json(json!({
        "status": if healthy { "ok" } else { "dégradé" },
        "version": env!("CARGO_PKG_VERSION"),
        "database": database,
    }))
}

#[get("/api/stats")]
pub async fn stats(app: web::Data<AppState>) -> HttpResponse {
    let mut database = json!({ "enabled": false });
    if let Some(repository) = app.jobs.database() {
        database = match repository.inventory().await {
            Ok(inventory) => json!({
                "enabled": true,
                "archives": inventory.archives,
                "oligos": inventory.oligos,
            }),
            Err(error) => json!({ "enabled": true, "error": error.to_string() }),
        };
    }
    HttpResponse::Ok().json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": app.started.to_string(),
        "uptime_seconds": (Timestamp::now() - app.started).as_secs_f64(),
        "limits": {
            "upload_bytes": app.limits.upload_bytes,
            "concurrent_jobs": app.limits.concurrent_jobs,
            "retention_minutes": app.limits.retention_minutes,
        },
        "schemes": adn_core::Scheme::ALL,
        "jobs": app.jobs.counts(),
        "database": database,
    }))
}

// Construction de l'application
// ------------------------------

/// L'application complète : routes et pages, puis ses intergiciels.
///
/// Une macro, parce que le type d'une application (même non enveloppée) ne s'écrit pas :
/// l'expression s'étend là où elle sert, dans le serveur comme dans les tests.
///
/// - `$state` : l'état partagé ([`web::Data<AppState>`]) ;
/// - `$static_files` : le dossier servi sous `/static` ;
/// - `$policy` : les hôtes que les requêtes peuvent désigner ([`HostPolicy`]).
#[macro_export]
macro_rules! app {
    ($state:expr, $static_files:expr, $policy:expr) => {{
        let state = $state;
        let policy = $policy;
        ::actix_web::App::new()
            .app_data(state.clone())
            // Les extracteurs (dont `Multipart`) ont par défaut une limite de 256 Kio : elle doit
            // couvrir un fichier accepté et l'enveloppe du formulaire.
            .app_data(::actix_web::web::PayloadConfig::new(
                state.limits.upload_bytes.saturating_add(64 * 1024),
            ))
            .service($crate::routes::index)
            .service($crate::routes::encode_page)
            .service($crate::routes::decode_page)
            .service($crate::routes::api_encode)
            .service($crate::routes::api_decode)
            .service($crate::routes::job_list)
            .service($crate::routes::job_status)
            .service($crate::routes::job_delete)
            .service($crate::routes::download_fasta)
            .service($crate::routes::download_file)
            .service($crate::routes::health)
            .service($crate::routes::stats)
            // Sans liste de dossier : seuls les fichiers référencés par les pages sont servis.
            .service(::actix_files::Files::new("/static", $static_files))
            // Les fichiers restitués sont d'origine inconnue : ils partent en pièce jointe
            // (voir `attachment`) et ne sont jamais interprétés comme une page de ce site.
            .wrap(
                ::actix_web::middleware::DefaultHeaders::new()
                    .add((::actix_web::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
                    .add((::actix_web::http::header::X_FRAME_OPTIONS, "DENY"))
                    .add((::actix_web::http::header::REFERRER_POLICY, "same-origin"))
                    .add((
                        ::actix_web::http::header::CONTENT_SECURITY_POLICY,
                        "frame-ancestors 'none'",
                    )),
            )
            .wrap(::actix_web::middleware::from_fn(move |request, next| {
                // Chaque requête emporte sa copie : rien de la closure n'est emprunté après coup.
                $crate::routes::guard_local(request, next, policy.clone())
            }))
    }};
}

/// Noms d'hôtes qu'une requête peut désigner.
///
/// Un site tiers peut faire rebinder son domaine vers `127.0.0.1` : le navigateur le traite
/// alors comme une origine de confiance et lit les réponses du serveur local. Le serveur ne
/// prête donc aucune prise à cette attaque : il n'accepte que des hôtes qu'on lui a désignés.
///
/// - La boucle locale est toujours acceptée.
/// - Un serveur lié à la boucle locale, sans autre hôte déclaré, n'accepte qu'elle.
/// - Un serveur publié, sans hôte déclaré, accepte tous les hôtes : c'est le rôle d'un proxy.
/// - Dès qu'un hôte est déclaré, lui seul (avec la boucle locale) est accepté.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPolicy {
    trusted: Vec<String>,
    loopback_only: bool,
}

impl HostPolicy {
    /// Politique d'un serveur lié à `host`, qui accepte en plus les noms de `trusted`.
    pub fn new(host: &str, trusted: Vec<String>) -> Self {
        let trusted = trusted
            .into_iter()
            .map(|name| name.trim().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .collect();
        Self {
            trusted,
            loopback_only: crate::is_loopback(host),
        }
    }

    /// Serveur d'essai lié à la boucle locale, sans autre hôte déclaré.
    pub const LOCAL: Self = Self {
        trusted: Vec::new(),
        loopback_only: true,
    };

    /// Serveur d'essai publié, sans hôte déclaré.
    pub const ANY: Self = Self {
        trusted: Vec::new(),
        loopback_only: false,
    };

    /// Vrai si une requête peut désigner `host` : le contenu de l'en-tête `Host`.
    pub fn accepts(&self, host: &str) -> bool {
        // `[::1]` ou `[::1]:8080` pour IPv6, `hôte:port` ou `hôte` sinon.
        let name = match host.find(']') {
            Some(end) => &host[..=end],
            None => host.rsplit_once(':').map_or(host, |(name, _)| name),
        };
        if crate::is_loopback(name) {
            return true;
        }
        let name = name
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        if self.trusted.is_empty() {
            return !self.loopback_only;
        }
        self.trusted.contains(&name)
    }
}

/// Intergiciel ajouté par [`app!`](crate::app) : refuse les requêtes dont l'hôte n'est pas
/// accepté par `policy`.
pub async fn guard_local(
    request: actix_web::dev::ServiceRequest,
    next: actix_web::middleware::Next<actix_web::body::BoxBody>,
    policy: HostPolicy,
) -> actix_web::Result<actix_web::dev::ServiceResponse> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if policy.accepts(host) {
        return next.call(request).await;
    }
    let shown = if host.is_empty() { "absent" } else { host };
    warn!("Requête refusée : l'hôte demandé est « {shown} »");
    let refused = HttpResponse::Forbidden().json(json!({ "error": "hôte refusé" }));
    Ok(request.into_response(refused))
}

// Formulaires
// -----------

/// Ce qu'un formulaire multipart apporte.
struct Upload {
    file_name: String,
    data: Vec<u8>,
    fields: Vec<(String, String)>,
}

/// Lit un formulaire multipart entier. Il tient en mémoire : la taille des fichiers est bornée
/// par `limit`.
// Une réponse d'erreur ne voyage qu'une fois, à l'échec : la boxer ne gagnerait rien.
#[allow(clippy::result_large_err)]
async fn read_upload(payload: &mut Multipart, limit: usize) -> Result<Upload, HttpResponse> {
    let mut upload = Upload {
        file_name: String::new(),
        data: Vec::new(),
        fields: Vec::new(),
    };
    let mut files = 0;
    while let Some(item) = payload.next().await {
        let mut field = item.map_err(|error| {
            warn!("Formulaire illisible : {error}");
            refusal(StatusCode::BAD_REQUEST, "formulaire illisible")
        })?;
        let (name, file_name) = {
            // Les champs d'un formulaire sans en-tête de disposition sont lus comme du texte.
            let disposition = field.content_disposition();
            let name = disposition
                .and_then(|disposition| disposition.get_name())
                .unwrap_or_default()
                .to_string();
            let file_name = disposition
                .and_then(|disposition| disposition.get_filename())
                .map(safe_file_name)
                .unwrap_or_default();
            (name, file_name)
        };
        let is_file = !file_name.is_empty();
        let cap = if is_file { limit } else { FIELD_LIMIT };
        let mut data = Vec::new();
        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|error| {
                warn!("Formulaire interrompu : {error}");
                refusal(StatusCode::BAD_REQUEST, "formulaire interrompu")
            })?;
            if data.len() + chunk.len() > cap {
                return Err(if is_file {
                    refusal(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!(
                            "fichier trop volumineux : la limite est de {} ({limit} octets)",
                            human_bytes(limit as u64)
                        ),
                    )
                } else {
                    refusal(
                        StatusCode::BAD_REQUEST,
                        format!("champ « {name} » trop long"),
                    )
                });
            }
            data.extend_from_slice(&chunk);
        }
        if is_file {
            files += 1;
            if files > 1 {
                return Err(refusal(
                    StatusCode::BAD_REQUEST,
                    "un seul fichier par requête",
                ));
            }
            upload.file_name = file_name;
            upload.data = data;
        } else {
            upload
                .fields
                .push((name, String::from_utf8_lossy(&data).trim().to_string()));
        }
    }
    Ok(upload)
}

// Une réponse d'erreur ne voyage qu'une fois, à l'échec : la boxer ne gagnerait rien.
#[allow(clippy::result_large_err)]
fn encode_options(fields: &[(String, String)]) -> Result<EncodeOptions, HttpResponse> {
    let mut options = EncodeOptions::default();
    for (name, value) in fields {
        let value = value.trim();
        match name.trim() {
            // Un champ vide ou absent garde la valeur par défaut.
            "scheme" if !value.is_empty() => {
                options.scheme = adn_core::codec::parse_scheme(value)
                    .map_err(|error| refusal(StatusCode::BAD_REQUEST, error.to_string()))?
            }
            "compression" if !value.is_empty() => {
                options.compression = match value.to_ascii_lowercase().as_str() {
                    "auto" => Compression::Auto,
                    "none" => Compression::None,
                    "lz4" => Compression::Lz4,
                    "zstd" => Compression::Zstd,
                    other => {
                        return Err(refusal(
                            StatusCode::BAD_REQUEST,
                            format!("compression inconnue : « {other} » (auto, none, lz4 ou zstd)"),
                        ))
                    }
                }
            }
            "redundancy" if !value.is_empty() => {
                options.fountain.redundancy = value.parse().map_err(|_| {
                    refusal(
                        StatusCode::BAD_REQUEST,
                        format!("la redondance doit être un nombre (reçu « {value} »)"),
                    )
                })?
            }
            "scheme" | "compression" | "redundancy" => {}
            other => {
                return Err(refusal(
                    StatusCode::BAD_REQUEST,
                    format!("champ inconnu : « {other} »"),
                ))
            }
        }
    }
    options
        .validate()
        .map_err(|error| refusal(StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(options)
}

/// Refuse une nouvelle tâche si trop d'autres attendent déjà.
fn queue_full(app: &AppState) -> Option<HttpResponse> {
    let pending = app.jobs.pending();
    (pending >= app.limits.concurrent_jobs * QUEUE_PER_WORKER).then(|| {
        warn!("File pleine : {pending} tâches en attente");
        refusal(
            StatusCode::TOO_MANY_REQUESTS,
            "trop de tâches en attente : réessayer dans un instant",
        )
    })
}

fn accepted(job_id: &str) -> HttpResponse {
    HttpResponse::Accepted().json(Accepted {
        job_id: job_id.to_string(),
        status: JobStatus::Queued,
        location: format!("/api/jobs/{job_id}"),
    })
}

/// Une réponse refusée, toujours en JSON avec un message.
fn refusal(status: StatusCode, message: impl Into<String>) -> HttpResponse {
    let message = message.into();
    HttpResponse::build(status).json(json!({ "error": message, "code": status.as_u16() }))
}

// Noms et en-têtes
// ----------------

/// Nom de fichier proposé par l'utilisateur, rendu inoffensif : sans chemin, sans caractères
/// de contrôle ni caractères refusés par les systèmes de fichiers. Vide s'il ne reste rien.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(character, '"' | '<' | '>' | '|' | '?' | '*' | ':' | ';')
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let cleaned: String = cleaned.chars().take(FILE_NAME_LIMIT).collect();
    cleaned.trim().trim_matches('.').to_string()
}

/// Valeur de l'en-tête `Content-Disposition` d'un téléchargement : un nom de repli en ASCII et
/// le nom exact, encodé pour les navigateurs qui savent le lire (RFC 8187).
pub fn attachment(file_name: &str) -> String {
    let name = match safe_file_name(file_name) {
        name if name.is_empty() => UNNAMED_FILE.to_string(),
        name => name,
    };
    let fallback: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() && !matches!(character, '"' | '\\') {
                character
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
        percent_encode(&name)
    )
}

/// Encode tout ce qui n'est pas un caractère sans réserve d'une URL (RFC 3986).
fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            write!(encoded, "%{byte:02X}").expect("écrire dans une chaîne");
        }
    }
    encoded
}

/// Identifiant d'une tâche, tel qu'il figure dans l'URL. Il sert à construire un chemin de
/// fichier : seules les 36 lettres et chiffres d'un UUID sont acceptées.
// Une réponse d'erreur ne voyage qu'une fois, à l'échec : la boxer ne gagnerait rien.
#[allow(clippy::result_large_err)]
fn job_id(raw: &str) -> Result<String, HttpResponse> {
    match Uuid::parse_str(raw) {
        Ok(uuid) if raw.len() == 36 && uuid.to_string().eq_ignore_ascii_case(raw) => {
            Ok(uuid.to_string())
        }
        _ => Err(refusal(
            StatusCode::BAD_REQUEST,
            "identifiant de tâche invalide",
        )),
    }
}

/// Taille lisible : `1,5 Mio`.
pub fn human_bytes(bytes: u64) -> String {
    const UNIT: f64 = 1024.0;
    let (value, unit) = match bytes {
        bytes if bytes < 1 << 10 => (bytes as f64, "octets"),
        bytes if bytes < 1 << 20 => (bytes as f64 / UNIT, "Kio"),
        bytes if bytes < 1 << 30 => (bytes as f64 / UNIT / UNIT, "Mio"),
        bytes => (bytes as f64 / UNIT / UNIT / UNIT, "Gio"),
    };
    if bytes == 1 {
        "1 octet".to_string()
    } else if value.fract() == 0.0 {
        format!("{value:.0} {unit}")
    } else {
        format!("{value:.1} {unit}").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_lose_paths_and_unsafe_characters() {
        assert_eq!(safe_file_name("lettre.txt"), "lettre.txt");
        assert_eq!(safe_file_name("photo.jpg"), "photo.jpg");
        assert_eq!(safe_file_name("données été.csv"), "données été.csv");
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\Users\\moi\\fichier.bin"), "fichier.bin");
        assert_eq!(safe_file_name("dossier/"), "");
        assert_eq!(
            safe_file_name("a\"b\r\nX-Injected: 1.txt"),
            "abX-Injected 1.txt"
        );
        assert_eq!(safe_file_name("..."), "");
        assert_eq!(safe_file_name(".cache"), "cache");
        assert_eq!(safe_file_name("fin ."), "fin");
        assert_eq!(safe_file_name(""), "");
        let long = "a".repeat(500);
        assert_eq!(safe_file_name(&long).chars().count(), FILE_NAME_LIMIT);
    }

    #[test]
    fn attachments_carry_an_ascii_name_and_the_exact_one() {
        assert_eq!(
            attachment("lettre.txt.fasta"),
            "attachment; filename=\"lettre.txt.fasta\"; filename*=UTF-8''lettre.txt.fasta"
        );
        assert_eq!(
            attachment("données été.csv"),
            "attachment; filename=\"donn_es__t_.csv\"; filename*=UTF-8''donn%C3%A9es%20%C3%A9t%C3%A9.csv"
        );
        assert_eq!(
            attachment("../../x.txt"),
            "attachment; filename=\"x.txt\"; filename*=UTF-8''x.txt"
        );
        // Une valeur d'en-tête doit rester un texte ASCII imprimable. Le barre oblique inverse
        // est un séparateur de chemin : il ne reste que ce qui le suit.
        let value = attachment("a\"b\\\r\né");
        assert_eq!(value, "attachment; filename=\"_\"; filename*=UTF-8''%C3%A9");
        header::HeaderValue::from_str(&value).expect("en-tête valide");
        assert_eq!(
            attachment("..."),
            format!("attachment; filename=\"{UNNAMED_FILE}\"; filename*=UTF-8''{UNNAMED_FILE}")
        );
        assert!(percent_encode("a-b_c.d~e é/\"").starts_with("a-b_c.d~e"));
    }

    #[test]
    fn encode_options_follow_the_form_and_refuse_the_rest() {
        let field = |name: &str, value: &str| (name.to_string(), value.to_string());
        let defaults = encode_options(&[]).unwrap();
        assert_eq!(defaults, EncodeOptions::default());

        let options = encode_options(&[
            field("scheme", "strands"),
            field("compression", "zstd"),
            field("redundancy", "2"),
        ])
        .unwrap();
        assert_eq!(options.scheme, adn_core::Scheme::Strands);
        assert_eq!(options.compression, Compression::Zstd);
        assert_eq!(options.fountain.redundancy, 2.0);

        // Les champs vides gardent les valeurs par défaut.
        let options = encode_options(&[field("scheme", ""), field("redundancy", " ")]).unwrap();
        assert_eq!(options, EncodeOptions::default());

        let refused = |fields: Vec<(String, String)>| encode_options(&fields).unwrap_err().status();
        assert_eq!(
            refused(vec![field("scheme", "ultimate")]),
            StatusCode::BAD_REQUEST,
            "un schéma retiré est refusé, pas ignoré"
        );
        // Les schémas de la littérature et le schéma ldpc sont proposés, avec leurs paramètres
        // par défaut.
        for name in ["goldman", "grass", "ldpc"] {
            let options = encode_options(&[field("scheme", name)]).unwrap();
            assert_eq!(options.scheme.name(), name);
            assert_eq!(options, EncodeOptions::for_scheme(options.scheme));
        }
        assert_eq!(
            refused(vec![field("scheme", "inconnu")]),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            refused(vec![field("compression", "zip")]),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            refused(vec![field("redundancy", "beaucoup")]),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            refused(vec![field("redundancy", "0.5")]),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            refused(vec![field("chunk_size", "32")]),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn only_uuids_name_jobs() {
        assert_eq!(
            job_id("0a1b2c3d-1111-4222-8333-444455556666").unwrap(),
            "0a1b2c3d-1111-4222-8333-444455556666"
        );
        // La casse est remise en place, sans changer l'identifiant.
        assert_eq!(
            job_id("0A1B2C3D-1111-4222-8333-444455556666").unwrap(),
            "0a1b2c3d-1111-4222-8333-444455556666"
        );
        for refused in [
            "../../etc/passwd",
            "..%2F..%2Fsecret",
            "aa\r\nbb",
            "",
            "not-a-uuid",
            "{0a1b2c3d-1111-4222-8333-444455556666}",
            "urn:uuid:0a1b2c3d-1111-4222-8333-444455556666",
            "0a1b2c3d111142228333444455556666",
        ] {
            assert_eq!(
                job_id(refused).unwrap_err().status(),
                StatusCode::BAD_REQUEST,
                "{refused}"
            );
        }
    }

    #[test]
    fn host_policies_follow_the_configuration() {
        for local in ["127.0.0.1:8080", "localhost", "[::1]", "127.5.4.3"] {
            assert!(HostPolicy::LOCAL.accepts(local), "{local}");
        }
        for refused in ["evil.example", "evil.example:8080", "192.168.1.20", ""] {
            assert!(!HostPolicy::LOCAL.accepts(refused), "{refused}");
            assert!(HostPolicy::ANY.accepts(refused), "{refused}");
        }

        // Les noms déclarés sont rognés, sans casse ni port ; les vides sont ignorés.
        let proxied = HostPolicy::new(
            "127.0.0.1",
            vec![" AdN.Example.ORG ".to_string(), " ".to_string()],
        );
        assert!(proxied.accepts("adn.example.org"));
        assert!(proxied.accepts("ADN.EXAMPLE.ORG:443"));
        assert!(proxied.accepts("localhost"));
        assert!(!proxied.accepts("evil.example.org"));

        // Dès qu'un nom est déclaré, lui seul passe, même sur un serveur publié.
        let published = HostPolicy::new("0.0.0.0", vec!["adn.example.org".to_string()]);
        assert!(published.accepts("adn.example.org"));
        assert!(!published.accepts("autre.example.org"));
        assert!(
            published.accepts("127.0.0.1"),
            "la boucle locale passe toujours"
        );
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_bytes(1), "1 octet");
        assert_eq!(human_bytes(2), "2 octets");
        assert_eq!(human_bytes(1024), "1 Kio");
        assert_eq!(human_bytes(1536), "1,5 Kio");
        assert_eq!(human_bytes(DEFAULT_LIMIT as u64), "25 Mio");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5 Gio");
    }

    const DEFAULT_LIMIT: usize = 25 * 1024 * 1024;
}
