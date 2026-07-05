use actix_web::middleware::from_fn;
use actix_web::web;

use super::auth::require_api_key;
use super::handlers;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(handlers::health))
        .route("/ready", web::get().to(handlers::ready))
        // 이미지 호스팅 공개 서빙 — 외부 공유/임베드를 위해 인증 없이 접근 가능
        .route("/media/{filename}", web::get().to(handlers::serve_media))
        .service(
            web::scope("/v1")
                .wrap(from_fn(require_api_key))
                .route("/images", web::post().to(handlers::upload_image))
                .route("/images", web::get().to(handlers::list_images))
                .route("/projects", web::post().to(handlers::create_project))
                .route("/projects", web::get().to(handlers::list_projects))
                .route("/projects/{id}", web::get().to(handlers::get_project))
                .route(
                    "/projects/{id}/architecture-answers",
                    web::post().to(handlers::submit_architecture_answers),
                )
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
