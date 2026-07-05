mod auth;
mod handlers;
mod routes;

use crate::app::App;
use crate::shutdown;
use actix_cors::Cors;
use actix_session::config::PersistentSession;
use actix_session::storage::CookieSessionStore;
use actix_session::SessionMiddleware;
use actix_web::cookie::SameSite;
use actix_web::{web, App as ActixApp, HttpServer};
use std::sync::Arc;
use tracing_actix_web::TracingLogger;

pub use routes::configure;

const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

fn build_cors(app: &App) -> Cors {
    let session_login = app.config.session_login_enabled();

    match &app.config.cors_allowed_origins {
        Some(origins) if !origins.is_empty() => {
            let mut cors = Cors::default()
                .allow_any_method()
                .allow_any_header()
                .max_age(3600);
            if session_login {
                cors = cors.supports_credentials();
            }
            for origin in origins {
                cors = cors.allowed_origin(origin);
            }
            cors
        }
        _ if session_login => {
            tracing::warn!(
                "CORS_ALLOWED_ORIGINS is not set — allowing local dev origins with credentials for session login"
            );
            Cors::default()
                .allowed_origin("http://localhost:5173")
                .allowed_origin("http://localhost:8080")
                .allowed_origin("http://127.0.0.1:5173")
                .allowed_origin("http://127.0.0.1:8080")
                .allow_any_method()
                .allow_any_header()
                .supports_credentials()
                .max_age(3600)
        }
        _ => {
            tracing::warn!(
                "CORS_ALLOWED_ORIGINS is not set — allowing any origin (recommended only for local development)"
            );
            Cors::default()
                .allow_any_origin()
                .allow_any_method()
                .allow_any_header()
                .max_age(3600)
        }
    }
}

pub async fn serve(app: Arc<App>) -> std::io::Result<()> {
    app.config.validate_and_warn();
    let bind = app.config.bind_addr();
    let data = web::Data::new(app.clone());
    let session_secret = app.config.session_secret.clone();

    tracing::info!(bind = %bind, session_login = app.config.session_login_enabled(), "starting API server");

    let server = HttpServer::new(move || {
        let secret = session_secret
            .as_deref()
            .unwrap_or("autoforge-dev-session-not-for-production");
        let key = auth::session_signing_key(secret);

        ActixApp::new()
            .wrap(TracingLogger::default())
            .wrap(build_cors(&data))
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), key)
                    .cookie_name("autoforge_session".to_string())
                    .cookie_http_only(true)
                    .cookie_same_site(SameSite::Lax)
                    .cookie_path("/".to_string())
                    .session_lifecycle(
                        PersistentSession::default()
                            .session_ttl(actix_web::cookie::time::Duration::days(7)),
                    )
                    .build(),
            )
            .app_data(data.clone())
            .app_data(web::PayloadConfig::new(data.config.max_upload_bytes))
            .configure(routes::configure)
    })
    .bind(&bind)?
    .shutdown_timeout(SHUTDOWN_TIMEOUT_SECS)
    .run();

    let handle = server.handle();
    tokio::spawn(async move {
        shutdown::wait_for_shutdown().await;
        tracing::info!("stopping API server (graceful shutdown)");
        handle.stop(true).await;
    });

    server.await
}
