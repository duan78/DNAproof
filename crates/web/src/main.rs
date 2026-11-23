//! Point d'entrée d'`adn-web`.
//!
//!     adn-web                 # lit `config.toml` à côté, ou les valeurs par défaut
//!     adn-web /etc/adn.toml   # lit la configuration indiquée
//!
//! Toute variable d'environnement `ADN_<SECTION>__<CLÉ>` survit au fichier (par exemple
//! `ADN_SERVER__HOST=0.0.0.0`).

#[actix_web::main]
async fn main() -> std::process::ExitCode {
    let path = adn_web::config_path(std::env::args_os().nth(1));
    let config = match adn_web::config::AppConfig::load(&path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "adn-web : configuration inapplicable ({}) : {error}",
                path.display()
            );
            return std::process::ExitCode::FAILURE;
        }
    };
    match adn_web::serve(config).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("adn-web : {message}");
            std::process::ExitCode::FAILURE
        }
    }
}
