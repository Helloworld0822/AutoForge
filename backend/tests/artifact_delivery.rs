use actix_web::{test, web, App as WebApp};
use autoforge::domain::{LanguageMode, PipelineModelConfig};
use autoforge::services::artifact_access::issue_coder_url;
use autoforge::{App, Config};
use bytes::Bytes;
use chrono::Utc;
use std::sync::Arc;

async fn build_app(secret: &str, root: &std::path::Path) -> Arc<App> {
    let mut config = Config::from_env();
    config.artifacts_dir = root.to_string_lossy().into_owned();
    config.artifact_signing_secret = Some(secret.into());
    config.public_url = "http://localhost".into();
    config.github_token = None;
    config.slack_webhook_url = None;
    config.slack_bot_token = None;
    config.git_auto_commit = false;
    App::new(config).await.expect("app").shared()
}

fn token_of(url: &str) -> &str {
    url.split("token=").nth(1).expect("token")
}

fn test_app(
    app: Arc<App>,
) -> WebApp<
    impl actix_web::dev::ServiceFactory<
        actix_web::dev::ServiceRequest,
        Config = (),
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    WebApp::new()
        .app_data(web::Data::new(app))
        .configure(autoforge::web::configure)
}

#[actix_web::test]
async fn serves_an_allowed_artifact_with_a_valid_token() {
    let root = std::env::temp_dir().join(format!("autoforge-artifact-{}", uuid::Uuid::new_v4()));
    let app = build_app("test-secret", &root).await;
    let project = app
        .create_project(
            None,
            None,
            None,
            LanguageMode::Auto,
            PipelineModelConfig::default(),
        )
        .await;

    let key = format!("projects/{}/architect/architecture.md", project.id.0);
    let artifact = app
        .artifacts
        .put(&key, Bytes::from("# Architecture"), "text/markdown")
        .await
        .expect("put artifact");

    let url = issue_coder_url(
        "test-secret",
        "http://localhost",
        project.id.0,
        &artifact,
        Utc::now(),
    )
    .expect("issue url");

    let app_service = test::init_service(test_app(app)).await;
    let req = test::TestRequest::get()
        .uri(&format!("/artifacts/coder?token={}", token_of(&url)))
        .to_request();
    let resp = test::call_service(&app_service, req).await;
    assert_eq!(resp.status(), 200);
    let body = test::read_body(resp).await;
    assert_eq!(&body[..], b"# Architecture");

    std::fs::remove_dir_all(root).expect("remove fixture");
}

#[actix_web::test]
async fn rejects_missing_wrong_and_expired_tokens() {
    let root = std::env::temp_dir().join(format!("autoforge-artifact-{}", uuid::Uuid::new_v4()));
    let app = build_app("test-secret", &root).await;
    let project = app
        .create_project(
            None,
            None,
            None,
            LanguageMode::Auto,
            PipelineModelConfig::default(),
        )
        .await;
    let key = format!("projects/{}/architect/spec.md", project.id.0);
    let artifact = app
        .artifacts
        .put(&key, Bytes::from("# Spec"), "text/markdown")
        .await
        .expect("put artifact");

    let wrong_secret = issue_coder_url(
        "other-secret",
        "http://localhost",
        project.id.0,
        &artifact,
        Utc::now(),
    )
    .expect("issue");
    let expired = issue_coder_url(
        "test-secret",
        "http://localhost",
        project.id.0,
        &artifact,
        Utc::now() - chrono::Duration::hours(2),
    )
    .expect("issue");

    let app_service = test::init_service(test_app(app)).await;
    for uri in [
        "/artifacts/coder".to_string(),
        format!("/artifacts/coder?token={}", token_of(&wrong_secret)),
        format!("/artifacts/coder?token={}", token_of(&expired)),
    ] {
        let req = test::TestRequest::get().uri(&uri).to_request();
        let resp = test::call_service(&app_service, req).await;
        assert!(
            resp.status().is_client_error(),
            "expected rejection for {uri}"
        );
    }

    std::fs::remove_dir_all(root).expect("remove fixture");
}
