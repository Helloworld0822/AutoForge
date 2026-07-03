use crate::app::App;
use actix_web::body::{EitherBody, MessageBody};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{http::header, web, Error, HttpResponse};
use std::sync::Arc;

/// `API_KEY`가 설정된 경우 `/v1/*` 요청에 `Authorization: Bearer <key>` 헤더를 요구한다.
/// 설정되어 있지 않으면(개발 모드) 통과시킨다.
pub async fn require_api_key<B: MessageBody + 'static>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    let expected_key = req
        .app_data::<web::Data<Arc<App>>>()
        .and_then(|app| app.config.api_key.clone());

    let Some(expected_key) = expected_key else {
        return next.call(req).await.map(|res| res.map_into_left_body());
    };

    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);

    if provided != Some(expected_key.as_str()) {
        let response = HttpResponse::Unauthorized()
            .json(serde_json::json!({ "error": "unauthorized: missing or invalid API key" }))
            .map_into_right_body();
        return Ok(req.into_response(response));
    }

    next.call(req).await.map(|res| res.map_into_left_body())
}
