//! Des requêtes réelles au serveur : pages, encodage, téléchargement, décodage de ce qui a été
//! téléchargé, refus, en-têtes, miroir en base de données.

use actix_http::Request;
use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::header::{self, HeaderMap};
use actix_web::http::StatusCode;
use actix_web::test;
use actix_web::web::{self, Bytes};
use adn_storage::{DatabaseConfig, DatabaseManager, SequenceRepository};
use adn_web::jobs::{JobKind, JobRegistry};
use adn_web::routes::{AppState, HostPolicy, Limits};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const CRATE: &str = env!("CARGO_MANIFEST_DIR");
static SERVERS: AtomicUsize = AtomicUsize::new(0);

/// Ce qu'un serveur d'essai laisse sur le disque : tout est sous `root`, jeté à la fin.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let rank = SERVERS.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("adn-web-api-{}-{rank}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("créer le dossier de test");
        Self { root }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Sous Windows, une base SQLite encore ouverte ne se laisse pas effacer : tant pis.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// L'état d'un serveur d'essai.
async fn state(
    scratch: &Scratch,
    upload_limit: usize,
    database: Option<DatabaseConfig>,
) -> web::Data<AppState> {
    let repository = match database {
        Some(config) => {
            let mut manager = DatabaseManager::new(config);
            manager.initialize().await.expect("ouvrir la base de test");
            Some(SequenceRepository::new(
                manager.shared_pool().expect("le pool est prêt"),
            ))
        }
        None => None,
    };
    let tera = adn_web::load_templates(&Path::new(CRATE).join("templates"))
        .expect("les gabarits du serveur");
    let registry = JobRegistry::new(scratch.root.join("uploads"), 2, repository)
        .expect("le dossier des résultats");
    web::Data::new(AppState {
        jobs: Arc::new(registry),
        tera,
        limits: Limits {
            upload_bytes: upload_limit,
            concurrent_jobs: 2,
            retention_minutes: 60,
        },
        started: jiff::Timestamp::now(),
    })
}

/// Une base SQLite dans le dossier de test.
fn sqlite(scratch: &Scratch) -> DatabaseConfig {
    DatabaseConfig::sqlite(scratch.root.join("base.db").to_string_lossy().into_owned())
}

/// Réponse d'un serveur d'essai.
#[derive(Debug)]
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("réponse sans JSON ({error}) : {:?}", self.body))
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    }
}

async fn send<S, B, E>(app: &S, request: Request) -> Reply
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    let response = app.call(request).await.expect("le serveur répond");
    Reply {
        status: response.status(),
        headers: response.headers().clone(),
        body: test::read_body(response).await,
    }
}

async fn get<S, B, E>(app: &S, path: &str) -> Reply
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    let request = test::TestRequest::get().uri(path).local().to_request();
    send(app, request).await
}

async fn get_with<S, B, E>(app: &S, path: &str, name: &str, value: &str) -> Reply
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    let request = test::TestRequest::get()
        .uri(path)
        .local()
        .insert_header((name, value))
        .to_request();
    send(app, request).await
}

async fn delete<S, B, E>(app: &S, path: &str) -> Reply
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    let request = test::TestRequest::delete().uri(path).local().to_request();
    send(app, request).await
}

/// Un hôte local, comme tout navigateur envoie sur la boucle locale.
trait Local {
    fn local(self) -> Self;
}

impl Local for test::TestRequest {
    fn local(self) -> Self {
        self.insert_header((header::HOST, "127.0.0.1:8080"))
    }
}

/// Un formulaire `multipart/form-data`, construit à la main.
struct Form {
    boundary: String,
    body: Vec<u8>,
}

impl Form {
    fn new() -> Self {
        Self {
            boundary: "adn-frontiere-de-test".to_string(),
            body: Vec::new(),
        }
    }

    fn part(mut self, name: &str, file_name: Option<&str>, content: &[u8]) -> Self {
        let mut head = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"{name}\"",
            self.boundary
        );
        if let Some(file_name) = file_name {
            head.push_str(&format!("; filename=\"{file_name}\""));
        }
        head.push_str("\r\n\r\n");
        self.body.extend_from_slice(head.as_bytes());
        self.body.extend_from_slice(content);
        self.body.extend_from_slice(b"\r\n");
        self
    }

    fn file(self, name: &str, file_name: &str, content: &[u8]) -> Self {
        self.part(name, Some(file_name), content)
    }

    fn field(self, name: &str, value: &str) -> Self {
        self.part(name, None, value.as_bytes())
    }
}

async fn post<S, B, E>(app: &S, path: &str, form: Form) -> Reply
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    let mut form = form;
    form.body
        .extend_from_slice(format!("--{}--\r\n", form.boundary).as_bytes());
    let request = test::TestRequest::post()
        .uri(path)
        .local()
        .insert_header((
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", form.boundary),
        ))
        .set_payload(form.body)
        .to_request();
    send(app, request).await
}

async fn encode<S, B, E>(
    app: &S,
    file_name: &str,
    content: &[u8],
    options: &[(&str, &str)],
) -> String
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    let mut form = Form::new().file("file", file_name, content);
    for (name, value) in options {
        form = form.field(name, value);
    }
    let reply = post(app, "/api/encode", form).await;
    assert_eq!(reply.status, StatusCode::ACCEPTED, "{}", reply.text());
    let accepted = reply.json();
    assert_eq!(accepted["status"], "queued");
    assert_eq!(
        accepted["location"],
        format!("/api/jobs/{}", accepted["job_id"].as_str().unwrap())
    );
    accepted["job_id"].as_str().unwrap().to_string()
}

/// Attend la fin d'une tâche et rend son état final.
async fn finished<S, B, E>(app: &S, job_id: &str) -> Value
where
    S: Service<Request, Response = ServiceResponse<B>, Error = E>,
    E: Debug,
    B: MessageBody,
{
    for _ in 0..3000 {
        let job = get(app, &format!("/api/jobs/{job_id}")).await.json();
        match job["status"].as_str() {
            Some("complete") | Some("failed") => return job,
            _ => actix_web::rt::time::sleep(Duration::from_millis(10)).await,
        }
    }
    panic!("la tâche {job_id} ne se termine pas");
}

const LETTER: &[u8] = b"Bonjour, ADN !\nCe fichier fait l'aller-retour par le serveur.\n";

#[actix_web::test]
async fn pages_and_static_files_are_served() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    for (path, marker) in [
        ("/", "Tâches récentes"),
        ("/encode", "Encoder un fichier en ADN"),
        ("/decode", "Décoder une archive"),
    ] {
        let page = get(&app, path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert!(
            page.header("content-type").starts_with("text/html"),
            "{path}"
        );
        let text = page.text();
        assert!(text.contains(marker), "{path} ne contient pas « {marker} »");
        assert!(text.contains("25 Mio"), "la limite d'envoi est affichée");
        // Un nom de fichier venu du serveur ne doit jamais arriver comme balise.
        assert!(text.contains("adn-upload-limit\" content=\"26214400\""));
    }
    assert!(get(&app, "/encode")
        .await
        .text()
        .contains("data-page=\"encode\""));
    assert!(get(&app, "/").await.text().contains("data-page=\"index\""));

    // En-têtes de sécurité, et pas de CORS : l'API ne se lit que depuis ses propres pages.
    let page = get(&app, "/").await;
    assert_eq!(page.header("x-content-type-options"), "nosniff");
    assert_eq!(page.header("x-frame-options"), "DENY");
    assert_eq!(page.header("referrer-policy"), "same-origin");
    let cross = get_with(&app, "/api/jobs", "origin", "https://ailleurs.example").await;
    assert!(cross.header("access-control-allow-origin").is_empty());

    let style = get(&app, "/static/css/app.css").await;
    assert_eq!(style.status, StatusCode::OK);
    assert!(style.header("content-type").starts_with("text/css"));
    assert_eq!(get(&app, "/static/js/app.js").await.status, StatusCode::OK);

    // Ni liste de dossier, ni fichier hors du dossier des pages.
    assert_eq!(get(&app, "/static/").await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        get(&app, "/static/css/").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/static/../Cargo.toml").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/uploads/x.fasta").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/templates/base.html").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(get(&app, "/absent").await.status, StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn a_file_round_trips_through_the_server() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state.clone(),
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    // Encodage.
    let encode_id = encode(&app, "lettre.txt", LETTER, &[]).await;
    let job = finished(&app, &encode_id).await;
    assert_eq!(job["status"], "complete", "{job}");
    assert_eq!(job["kind"], "encode");
    assert_eq!(job["file_name"], "lettre.txt.fasta");
    assert_eq!(
        job["sha256"],
        adn_core::checksum::hex(&adn_core::checksum::sha256(LETTER))
    );
    assert_eq!(job["download_url"], format!("/download/fasta/{encode_id}"));
    assert_eq!(
        job["stats"]["oligos"].as_u64().unwrap(),
        job["stats"]["data_oligos"].as_u64().unwrap()
            + job["stats"]["manifest_copies"].as_u64().unwrap()
    );
    assert!(job["stats"]["bits_per_nucleotide"].as_f64().unwrap() > 0.0);
    assert!(job["created_at"].is_string(), "les dates sont en RFC 3339");

    // Téléchargement de l'archive, qui décode vers le fichier d'origine.
    let archive = get(&app, &format!("/download/fasta/{encode_id}")).await;
    assert_eq!(archive.status, StatusCode::OK);
    assert_eq!(
        archive.header("content-disposition"),
        "attachment; filename=\"lettre.txt.fasta\"; filename*=UTF-8''lettre.txt.fasta"
    );
    assert!(archive.header("content-type").starts_with("text/plain"));
    let text = archive.text();
    assert!(text.starts_with(">adn-fountain-v2 manifest=0\n"), "{text}");
    let reads = adn_core::fasta::read_reader(text.as_bytes()).expect("archive FASTA valide");
    let decoded = adn_core::decode(
        reads.into_iter().map(|record| record.sequence),
        &adn_core::DecodeOptions::default(),
    )
    .expect("l'archive téléchargée se décode");
    assert_eq!(decoded.data, LETTER);

    // Décodage de ce même fichier, renvoyé au serveur.
    let form = Form::new().file("file", "lettre.txt.fasta", archive.body.as_ref());
    let accepted = post(&app, "/api/decode", form).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text());
    let decode_id = accepted.json()["job_id"].as_str().unwrap().to_string();
    let job = finished(&app, &decode_id).await;
    assert_eq!(job["status"], "complete", "{job}");
    assert_eq!(job["file_name"], "lettre.txt");
    assert_eq!(job["size"], LETTER.len());
    assert_eq!(
        job["sha256"],
        adn_core::checksum::hex(&adn_core::checksum::sha256(LETTER))
    );
    assert_eq!(job["stats"]["scheme"], "fountain");
    assert_eq!(job["download_url"], format!("/download/{decode_id}"));

    let file = get(&app, &format!("/download/{decode_id}")).await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(
        file.header("content-disposition"),
        "attachment; filename=\"lettre.txt\"; filename*=UTF-8''lettre.txt"
    );
    assert_eq!(file.header("content-type"), "application/octet-stream");
    assert_eq!(file.body.as_ref(), LETTER);

    // Chaque téléchargement ne sert que son espèce de résultat.
    assert_eq!(
        get(&app, &format!("/download/{encode_id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &format!("/download/fasta/{decode_id}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // Liste et état du serveur.
    let listed: Vec<String> = get(&app, "/api/jobs").await.json()["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(&encode_id) && listed.contains(&decode_id));

    let stats = get(&app, "/api/stats").await.json();
    assert_eq!(stats["jobs"]["complete"], 2);
    assert_eq!(stats["limits"]["upload_bytes"], 25 * 1024 * 1024);
    assert_eq!(stats["database"]["enabled"], false);
    assert_eq!(stats["schemes"], json!(["fountain", "strands"]));

    let health = get(&app, "/health").await;
    assert_eq!(health.status, StatusCode::OK);
    assert_eq!(health.json()["status"], "ok");
}

#[actix_web::test]
async fn encoding_options_are_honoured() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    let strands = encode(
        &app,
        "binaire.bin",
        &[0u8, 255, 7, 7, 7, 42],
        &[("scheme", "strands"), ("compression", "none")],
    )
    .await;
    let job = finished(&app, &strands).await;
    assert_eq!(job["status"], "complete", "{job}");
    assert_eq!(job["stats"]["scheme"], "strands");
    assert_eq!(job["stats"]["compression"], "None");
    assert_eq!(job["stats"]["oligo_length_nt"], 160);
    assert_eq!(job["file_name"], "binaire.bin.fasta");

    let fountain = encode(
        &app,
        "b.txt",
        &b"x".repeat(5000),
        &[("redundancy", "2"), ("compression", "lz4")],
    )
    .await;
    let job = finished(&app, &fountain).await;
    assert_eq!(job["status"], "complete", "{job}");
    assert_eq!(job["stats"]["scheme"], "fountain");
    assert_eq!(job["stats"]["redundancy"], 2.0);
    assert_eq!(job["stats"]["compression"], "Lz4");
    assert_eq!(job["stats"]["oligo_length_nt"], 152);

    // Les deux archives se décodent sans que le schéma soit indiqué.
    for (id, content) in [
        (&strands, vec![0u8, 255, 7, 7, 7, 42]),
        (&fountain, b"x".repeat(5000)),
    ] {
        let archive = get(&app, &format!("/download/fasta/{id}")).await.body;
        let form = Form::new().file("file", "archive.fa", archive.as_ref());
        let accepted = post(&app, "/api/decode", form).await;
        let id = accepted.json()["job_id"].as_str().unwrap().to_string();
        let job = finished(&app, &id).await;
        assert_eq!(job["status"], "complete", "{job}");
        assert_eq!(job["file_name"], "archive");
        let file = get(&app, &format!("/download/{id}")).await.body;
        assert_eq!(file.as_ref(), content.as_slice());
    }
}

#[actix_web::test]
async fn bad_requests_are_refused_before_a_job_exists() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    let cases: Vec<(Form, StatusCode, &str)> = vec![
        (Form::new(), StatusCode::BAD_REQUEST, "aucun fichier reçu"),
        (
            Form::new().field("scheme", "fountain"),
            StatusCode::BAD_REQUEST,
            "aucun fichier reçu",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .field("scheme", "goldman"),
            StatusCode::BAD_REQUEST,
            "goldman",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .field("scheme", "inconnu"),
            StatusCode::BAD_REQUEST,
            "inconnu",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .field("compression", "zip"),
            StatusCode::BAD_REQUEST,
            "compression inconnue",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .field("redundancy", "beaucoup"),
            StatusCode::BAD_REQUEST,
            "la redondance doit être un nombre",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .field("redundancy", "0.5"),
            StatusCode::BAD_REQUEST,
            "la redondance doit être un nombre fini entre 1.0 et 100.0",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .field("chunk_size", "32"),
            StatusCode::BAD_REQUEST,
            "champ inconnu",
        ),
        (
            Form::new()
                .file("file", "a.txt", b"x")
                .file("file", "b.txt", b"y"),
            StatusCode::BAD_REQUEST,
            "un seul fichier par requête",
        ),
    ];
    for (form, status, needle) in cases {
        let reply = post(&app, "/api/encode", form).await;
        assert_eq!(reply.status, status, "{}", reply.text());
        assert!(
            reply.json()["error"].as_str().unwrap().contains(needle),
            "{}",
            reply.text()
        );
    }

    let cases = vec![
        (
            Form::new().field("scheme", "fountain"),
            StatusCode::BAD_REQUEST,
            "aucune archive reçue",
        ),
        (
            Form::new()
                .file("file", "a.fa", b"ACGT")
                .field("expected_sha256", "zz"),
            StatusCode::BAD_REQUEST,
            "64 chiffres hexadécimaux",
        ),
        (
            Form::new()
                .file("file", "a.fa", b"ACGT")
                .field("scheme", "strands"),
            StatusCode::BAD_REQUEST,
            "champ inconnu",
        ),
    ];
    for (form, status, needle) in cases {
        let reply = post(&app, "/api/decode", form).await;
        assert_eq!(reply.status, status, "{}", reply.text());
        assert!(
            reply.json()["error"].as_str().unwrap().contains(needle),
            "{}",
            reply.text()
        );
    }

    // Aucune tâche n'a été créée par ces refus.
    assert_eq!(get(&app, "/api/jobs").await.json()["jobs"], json!([]));

    // Identifiants mal formés : jamais un chemin de fichier. Ceux qui n'atteignent même pas
    // une route donnent 404, les autres sont refusés.
    for path in [
        "/api/jobs/../../etc/passwd",
        "/api/jobs/not-a-uuid",
        "/api/jobs/{0a1b2c3d-1111-4222-8333-444455556666}",
        "/download/fasta/..%2F..%2Fsecret",
        "/download/aa",
    ] {
        // Un DELETE sur une route de téléchargement n'existe pas : 405, et rien n'est lu.
        let reply = get(&app, path).await;
        assert!(
            matches!(
                reply.status,
                StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
            ),
            "{path} : {} {}",
            reply.status,
            reply.text()
        );
        assert!(!reply.text().contains("root:"), "{path}");
        let reply = delete(&app, path).await;
        assert!(
            matches!(
                reply.status,
                StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{path} : {} {}",
            reply.status,
            reply.text()
        );
    }

    // Identifiants bien formés mais inconnus.
    let unknown = "0a1b2c3d-1111-4222-8333-444455556666";
    for path in [
        format!("/api/jobs/{unknown}"),
        format!("/download/{unknown}"),
        format!("/download/fasta/{unknown}"),
    ] {
        assert_eq!(
            get(&app, &path).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert_eq!(
        delete(&app, &format!("/api/jobs/{unknown}")).await.status,
        StatusCode::NOT_FOUND
    );
}

#[actix_web::test]
async fn an_oversized_file_is_refused() {
    let scratch = Scratch::new();
    let state = state(&scratch, 2048, None).await;
    let app = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    let page = get(&app, "/encode").await.text();
    assert!(
        page.contains("2 Kio"),
        "la page affiche la limite configurée"
    );

    let too_big = post(
        &app,
        "/api/encode",
        Form::new().file("file", "gros.bin", &vec![0u8; 4096]),
    )
    .await;
    assert_eq!(too_big.status, StatusCode::PAYLOAD_TOO_LARGE);
    let error = too_big.json()["error"].as_str().unwrap().to_string();
    assert!(error.contains("2 Kio") && error.contains("2048"), "{error}");

    let exactly = encode(&app, "juste.bin", &vec![7u8; 2048], &[]).await;
    assert_eq!(finished(&app, &exactly).await["status"], "complete");

    assert!(
        get(&app, "/api/jobs").await.json()["jobs"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
}

#[actix_web::test]
async fn a_bad_archive_fails_with_an_explanation() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    let decode = |file_name: &'static str, content: Vec<u8>, expected: Option<String>| {
        let app = &app;
        async move {
            let mut form = Form::new().file("file", file_name, &content);
            if let Some(expected) = &expected {
                form = form.field("expected_sha256", expected);
            }
            let accepted = post(app, "/api/decode", form).await;
            assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text());
            let id = accepted.json()["job_id"].as_str().unwrap().to_string();
            finished(app, &id).await
        }
    };

    // Du texte qui n'est pas une archive.
    let job = decode("notes.txt", b"Bonjour".to_vec(), None).await;
    assert_eq!(job["status"], "failed");
    let error = job["error"].as_str().unwrap();
    assert!(error.contains("archive illisible"), "{error}");
    assert!(
        job["download_url"].is_null(),
        "rien à télécharger pour un échec"
    );

    // Une archive de la version 0.1.
    let job = decode(
        "ancienne.fasta",
        b">seq1|scheme:fountain#4|seed:1\nACGTACGTAA\n".to_vec(),
        None,
    )
    .await;
    assert_eq!(job["status"], "failed");
    assert!(
        job["error"].as_str().unwrap().contains("version 0.1"),
        "{job}"
    );

    // Une archive vide.
    let job = decode("vide.fasta", b"".to_vec(), None).await;
    assert_eq!(job["status"], "failed");

    // Une empreinte attendue qui ne correspond pas : le fichier n'est pas rendu.
    let archive = {
        let encoded = adn_core::encode(LETTER, &adn_core::EncodeOptions::default()).unwrap();
        let mut text = String::new();
        for oligo in &encoded.oligos {
            text.push_str(">lecture\n");
            text.push_str(std::str::from_utf8(oligo).unwrap());
            text.push('\n');
        }
        text
    };
    let job = decode(
        "lettre.fasta",
        archive.clone().into_bytes(),
        Some("00".repeat(32)),
    )
    .await;
    assert_eq!(job["status"], "failed", "{job}");
    assert!(
        job["error"]
            .as_str()
            .unwrap()
            .contains("empreinte inattendue"),
        "{job}"
    );
    assert_eq!(
        get(&app, &format!("/download/{}", job["id"].as_str().unwrap()))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // La bonne empreinte est acceptée.
    let expected = adn_core::checksum::hex(&adn_core::checksum::sha256(LETTER));
    let job = decode("lettre.fasta", archive.into_bytes(), Some(expected)).await;
    assert_eq!(job["status"], "complete", "{job}");
}

#[actix_web::test]
async fn a_job_can_be_deleted() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state.clone(),
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    let id = encode(&app, "a-supprimer.txt", LETTER, &[]).await;
    assert_eq!(finished(&app, &id).await["status"], "complete");
    let path = state.jobs.result_path(&id, JobKind::Encode);
    assert!(path.is_file());

    // La méthode inconnue est refusée, sans effet.
    let request = test::TestRequest::put()
        .uri(&format!("/api/jobs/{id}"))
        .local()
        .to_request();
    assert_eq!(
        send(&app, request).await.status,
        StatusCode::METHOD_NOT_ALLOWED
    );

    let removed = delete(&app, &format!("/api/jobs/{id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(removed.body.is_empty());
    assert!(!path.exists(), "le résultat est effacé");
    assert_eq!(
        get(&app, &format!("/api/jobs/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &format!("/download/fasta/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        delete(&app, &format!("/api/jobs/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(get(&app, "/api/jobs").await.json()["jobs"], json!([]));

    // Une tâche en attente n'est pas annulable : elle reste, et se retire une fois terminée.
    let queued = state.jobs.submit(JobKind::Decode, None);
    let refused = delete(&app, &format!("/api/jobs/{}", queued.id)).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text());
    assert!(refused.json()["error"]
        .as_str()
        .unwrap()
        .contains("en cours"));
    assert_eq!(
        get(&app, &format!("/api/jobs/{}", queued.id)).await.status,
        StatusCode::OK
    );
    state.jobs.finish(&queued.id, Err("abandonnée".to_string()));
    assert_eq!(
        delete(&app, &format!("/api/jobs/{}", queued.id))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}

#[actix_web::test]
async fn only_local_hosts_are_served_unless_the_server_is_exposed() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let local = test::init_service(adn_web::app!(
        state.clone(),
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;
    let exposed = test::init_service(adn_web::app!(
        state.clone(),
        &Path::new(CRATE).join("static"),
        HostPolicy::ANY
    ))
    .await;
    // Derrière un proxy inverse qui préserve l'en-tête `Host`, le serveur reste lié à la boucle
    // locale et déclare le nom public du proxy.
    let proxied = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::new("127.0.0.1", vec!["adn.example.org".to_string()])
    ))
    .await;

    for host in ["127.0.0.1:8080", "localhost", "[::1]:8080", "127.5.4.3"] {
        let reply = get_with(&local, "/health", "host", host).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}");
    }
    // Sans en-tête, la requête HTTP/1.1 est mal formée : refusée aussi.
    for host in [
        "evil.example",
        "evil.example:8080",
        "192.168.1.20",
        "[2001:db8::1]:8080",
        "",
    ] {
        let reply = get_with(&local, "/health", "host", host).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{host}");
        assert_eq!(reply.json()["error"], "hôte refusé");
    }

    // Un serveur publié derrière un proxy reçoit d'autres noms : il les accepte.
    let reply = get_with(&exposed, "/health", "host", "adn.example.org").await;
    assert_eq!(reply.status, StatusCode::OK);

    // Seuls le nom déclaré et la boucle locale passent.
    for host in ["adn.example.org", "ADN.EXAMPLE.ORG:443", "localhost:8080"] {
        let reply = get_with(&proxied, "/health", "host", host).await;
        assert_eq!(reply.status, StatusCode::OK, "{host}");
    }
    let reply = get_with(&proxied, "/health", "host", "evil.example.org").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[actix_web::test]
async fn archives_are_mirrored_in_the_database_and_survive_a_restart() {
    let scratch = Scratch::new();
    let database = sqlite(&scratch);
    let state = state(&scratch, 25 * 1024 * 1024, Some(database.clone())).await;
    let app = test::init_service(adn_web::app!(
        state.clone(),
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    let id = encode(&app, "persistant.txt", LETTER, &[]).await;
    let job = finished(&app, &id).await;
    assert_eq!(job["status"], "complete", "{job}");
    assert_eq!(job["stats"]["stored_in_database"], true);

    let repository = state.jobs.database().expect("la base est activée");
    let archive = repository
        .archive(&id)
        .await
        .unwrap()
        .expect("l'archive est en base");
    assert_eq!(archive.info.file_name, "persistant.txt");
    assert_eq!(archive.info.scheme, "fountain");
    let stats: BTreeMap<String, Value> =
        serde_json::from_str(&archive.info.stats).expect("les mesures sont un objet JSON");
    assert_eq!(stats["oligos"], job["stats"]["oligos"]);
    assert_eq!(
        archive.oligos as u64,
        job["stats"]["oligos"].as_u64().unwrap()
    );
    let stats = get(&app, "/api/stats").await.json();
    assert_eq!(stats["database"]["enabled"], true);
    assert_eq!(stats["database"]["archives"], 1);
    let health = get(&app, "/health").await.json();
    assert_eq!(health["database"]["state"], "ok");

    // Le serveur redémarre : ses fichiers ont disparu, sa base est là.
    let file = state.jobs.result_path(&id, JobKind::Encode);
    std::fs::remove_file(&file).unwrap();
    let uploads = scratch.root.join("uploads-apres-redemarrage");
    let mut manager = DatabaseManager::new(database);
    manager.initialize().await.unwrap();
    let repository = SequenceRepository::new(manager.shared_pool().unwrap());
    let tera = adn_web::load_templates(&Path::new(CRATE).join("templates")).unwrap();
    let registry = Arc::new(JobRegistry::new(&uploads, 2, Some(repository)).unwrap());
    assert_eq!(registry.restore().await.unwrap(), 1);
    let state = web::Data::new(AppState {
        jobs: registry,
        tera,
        limits: Limits {
            upload_bytes: 25 * 1024 * 1024,
            concurrent_jobs: 2,
            retention_minutes: 60,
        },
        started: jiff::Timestamp::now(),
    });
    let app = test::init_service(adn_web::app!(
        state.clone(),
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;
    let job = get(&app, &format!("/api/jobs/{id}")).await.json();
    assert_eq!(job["status"], "complete", "{job}");
    assert_eq!(job["file_name"], "persistant.txt.fasta");
    let archive = get(&app, &format!("/download/fasta/{id}")).await;
    assert_eq!(archive.status, StatusCode::OK);
    let reads = adn_core::fasta::read_reader(archive.body.as_ref()).unwrap();
    assert_eq!(reads.len(), stored_oligos(&state, &id).await);
    let decoded = adn_core::decode(
        reads.into_iter().map(|record| record.sequence),
        &adn_core::DecodeOptions::default(),
    )
    .expect("l'archive réécrite depuis la base se décode");
    assert_eq!(decoded.data, LETTER);

    // Supprimer la tâche efface aussi la base.
    assert_eq!(
        delete(&app, &format!("/api/jobs/{id}")).await.status,
        StatusCode::NO_CONTENT
    );
    let repository = state.jobs.database().unwrap();
    assert!(repository.archive(&id).await.unwrap().is_none());
    assert_eq!(repository.inventory().await.unwrap().oligos, 0);
}

/// Nombre d'oligos rangés en base pour une archive.
async fn stored_oligos(state: &web::Data<AppState>, id: &str) -> usize {
    state
        .jobs
        .database()
        .unwrap()
        .archive(id)
        .await
        .unwrap()
        .unwrap()
        .oligos as usize
}

#[actix_web::test]
async fn a_failed_job_is_listed_with_its_explanation_and_can_be_removed() {
    let scratch = Scratch::new();
    let state = state(&scratch, 25 * 1024 * 1024, None).await;
    let app = test::init_service(adn_web::app!(
        state,
        &Path::new(CRATE).join("static"),
        HostPolicy::LOCAL
    ))
    .await;

    // Des lectures tronquées ne laissent pas reconstituer le fichier : la tâche échoue.
    let encoded = adn_core::encode(LETTER, &adn_core::EncodeOptions::default()).unwrap();
    let mut truncated = String::new();
    for oligo in encoded.oligos.iter().take(3) {
        truncated.push_str(">lecture\n");
        truncated.push_str(std::str::from_utf8(oligo).unwrap());
        truncated.push('\n');
    }
    let accepted = post(
        &app,
        "/api/decode",
        Form::new().file("file", "tronquee.fasta", truncated.as_bytes()),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    let id = accepted.json()["job_id"].as_str().unwrap().to_string();
    let job = finished(&app, &id).await;
    assert_eq!(job["status"], "failed", "{job}");
    let error = job["error"].as_str().unwrap();
    assert!(error.contains("décodage impossible"), "{error}");
    assert!(
        job["download_url"].is_null() && job["file_name"].is_null(),
        "{job}"
    );

    // L'échec est visible dans la liste, avec son explication.
    let listed = get(&app, "/api/jobs").await.json();
    assert_eq!(listed["jobs"][0]["status"], "failed");
    assert_eq!(listed["jobs"][0]["error"], error);
    assert_eq!(listed["jobs"][0]["source_name"], "tronquee.fasta");
    assert_eq!(get(&app, "/api/stats").await.json()["jobs"]["failed"], 1);

    // Le serveur reste utilisable, et la tâche échouée se retire.
    let good = encode(&app, "apres-echec.txt", LETTER, &[]).await;
    assert_eq!(finished(&app, &good).await["status"], "complete");
    assert_eq!(
        delete(&app, &format!("/api/jobs/{id}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        get(&app, "/api/jobs").await.json()["jobs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
