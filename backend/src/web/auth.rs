use crate::app::App;
use actix_session::Session;
use actix_session::SessionExt;
use actix_web::body::{EitherBody, MessageBody};
use actix_web::cookie::Key;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{web, Error, HttpResponse};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;

pub const SESSION_AUTH_KEY: &str = "authenticated";
pub const SESSION_USER_KEY: &str = "username";

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthMeResponse {
    pub authenticated: bool,
    pub username: Option<String>,
    pub session_login_enabled: bool,
    pub api_key_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub ok: bool,
    pub username: String,
}

pub fn session_signing_key(secret: &str) -> Key {
    let digest = Sha256::digest(secret.as_bytes());
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(&digest);
    bytes[32..].copy_from_slice(&digest);
    Key::from(&bytes)
}

fn credentials_match(provided: &str, expected: &str) -> bool {
    let provided_hash = Sha256::digest(provided.as_bytes());
    let expected_hash = Sha256::digest(expected.as_bytes());
    provided_hash.ct_eq(&expected_hash).into()
}

pub fn verify_login(
    username: &str,
    password: &str,
    expected_user: &str,
    expected_pass: &str,
) -> bool {
    credentials_match(username, expected_user) && credentials_match(password, expected_pass)
}

pub fn session_authenticated(session: &Session) -> bool {
    session
        .get::<bool>(SESSION_AUTH_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

fn auth_exempt_path(path: &str) -> bool {
    path == "/v1/auth/login" || path == "/v1/auth/me"
}

/// `API_KEY` 또는 유효한 세션 쿠키로 `/v1/*` 요청을 인증한다.
/// 둘 다 미설정이면(개발 모드) 통과시킨다.
pub async fn require_auth<B: MessageBody + 'static>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error> {
    let app = match req.app_data::<web::Data<Arc<App>>>() {
        Some(app) => app,
        None => return next.call(req).await.map(|res| res.map_into_left_body()),
    };

    let auth_required = app.config.auth_enabled();
    if !auth_required {
        return next.call(req).await.map(|res| res.map_into_left_body());
    }

    if auth_exempt_path(req.path()) {
        return next.call(req).await.map(|res| res.map_into_left_body());
    }

    let session = req.get_session();
    if app.config.session_login_enabled() && session_authenticated(&session) {
        return next.call(req).await.map(|res| res.map_into_left_body());
    }

    if let Some(expected_key) = app.config.api_key.as_deref() {
        let provided = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim);

        if provided == Some(expected_key) {
            return next.call(req).await.map(|res| res.map_into_left_body());
        }
    }

    let response = HttpResponse::Unauthorized()
        .json(serde_json::json!({ "error": "unauthorized: login or valid API key required" }))
        .map_into_right_body();
    Ok(req.into_response(response))
}

pub async fn login(
    app: web::Data<Arc<App>>,
    session: Session,
    body: web::Json<LoginRequest>,
) -> HttpResponse {
    if !app.config.session_login_enabled() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "session login is not configured"
        }));
    }

    let expected_user = app.config.login_username.as_deref().unwrap_or_default();
    let expected_pass = app.config.login_password.as_deref().unwrap_or_default();

    if !verify_login(
        body.username.trim(),
        &body.password,
        expected_user,
        expected_pass,
    ) {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "invalid username or password"
        }));
    }

    if let Err(err) = session.insert(SESSION_AUTH_KEY, true) {
        tracing::error!(error = %err, "failed to persist session");
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "failed to create session"
        }));
    }
    if let Err(err) = session.insert(SESSION_USER_KEY, expected_user) {
        tracing::error!(error = %err, "failed to persist session username");
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "failed to create session"
        }));
    }

    HttpResponse::Ok().json(LoginResponse {
        ok: true,
        username: expected_user.to_string(),
    })
}

pub async fn logout(session: Session) -> HttpResponse {
    session.purge();
    HttpResponse::Ok().json(serde_json::json!({ "ok": true }))
}

pub async fn auth_me(app: web::Data<Arc<App>>, session: Session) -> HttpResponse {
    let authenticated = app.config.session_login_enabled() && session_authenticated(&session);
    let username = if authenticated {
        session.get::<String>(SESSION_USER_KEY).ok().flatten()
    } else {
        None
    };

    HttpResponse::Ok().json(AuthMeResponse {
        authenticated,
        username,
        session_login_enabled: app.config.session_login_enabled(),
        api_key_enabled: app.config.api_key.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::verify_login;

    #[test]
    fn accepts_matching_credentials() {
        assert!(verify_login("admin", "secret", "admin", "secret"));
    }

    #[test]
    fn rejects_wrong_password() {
        assert!(!verify_login("admin", "wrong", "admin", "secret"));
    }

    #[test]
    fn rejects_wrong_username() {
        assert!(!verify_login("other", "secret", "admin", "secret"));
    }
}
