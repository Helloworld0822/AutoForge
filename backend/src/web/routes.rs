use actix_web::middleware::from_fn;
use actix_web::web;

use super::auth::require_api_key;
use super::handlers;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(handlers::health))
        .route("/ready", web::get().to(handlers::ready))
        .service(
            web::scope("/v1")
                .wrap(from_fn(require_api_key))
                .route("/projects", web::post().to(handlers::create_project))
                .route("/projects", web::get().to(handlers::list_projects))
                .route("/projects/{id}", web::get().to(handlers::get_project))
                .route(
                    "/projects/{id}/stream",
                    web::get().to(handlers::stream_project),
                )
                .route(
                    "/projects/{id}/cancel",
                    web::post().to(handlers::cancel_project),
                )
                .route(
                    "/projects/{id}/daily-logs",
                    web::get().to(handlers::list_daily_logs),
                )
                .route(
                    "/projects/{id}/daily-logs/{date}",
                    web::get().to(handlers::get_daily_log),
                ),
        );
}
