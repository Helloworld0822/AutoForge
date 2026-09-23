use actix_web::{web, App as WebApp, HttpResponse, HttpServer};
use autoforge::domain::{LanguageMode, PipelineModelConfig, StageId};
use autoforge::services::ai::ProjectSpec;
use autoforge::services::pipeline::engine::{apply_stage_output, execute_stage};
use autoforge::{App, Config};
use bytes::Bytes;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[actix_web::test]
async fn extraction_retries_schema_once_then_reuses_cache_offline() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = Arc::clone(&calls);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("mock listener");
    let address = listener.local_addr().expect("mock address");
    let server = HttpServer::new(move || {
        let calls = Arc::clone(&captured);
        WebApp::new().route("/v1/chat/completions", web::post().to(move |body: web::Json<serde_json::Value>| {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(body["model"], "fixture-extract");
                assert_eq!(body["max_tokens"], 16000);
                let source = body["messages"][1]["content"].as_str().expect("user message");
                assert!(source.contains("Deploy on Linux"));
                assert!(!source.contains("%PDF"));
                let spec = ProjectSpec { title: "API".into(), project_goal: "Serve inventory".into(), ..ProjectSpec::default() };
                let content = if attempt == 0 { "{}".into() } else { serde_json::to_string(&spec).expect("spec") };
                HttpResponse::Ok().json(serde_json::json!({
                    "model":"fixture-extract", "choices":[{"message":{"content":content},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":100,"completion_tokens":30,"cost":0.001}
                }))
            }
        }))
    }).listen(listener).expect("listen").run();
    let handle = server.handle();
    let server_task = actix_web::rt::spawn(server);
    let root = std::env::temp_dir().join(format!("autoforge-token-test-{}", uuid::Uuid::new_v4()));
    let mut config = Config::from_env();
    config.artifacts_dir = root.to_string_lossy().into_owned();
    config.omniroute_base_url = format!("http://{address}/v1");
    config.omniroute_api_key = "test-only".into();
    config.model_router.extract = "fixture-extract".into();
    config.github_token = None;
    config.slack_webhook_url = None;
    config.slack_bot_token = None;
    config.git_auto_commit = false;
    let app = App::new(config).await.expect("app");
    let mut project = app
        .create_project(
            None,
            None,
            None,
            LanguageMode::Auto,
            PipelineModelConfig::default(),
        )
        .await;
    for (name, source) in [
        ("raw_text.md", "# Inventory API\nUse Rust."),
        ("devops_raw_text.md", "Deploy on Linux"),
    ] {
        project.accumulated_artifacts.push(
            app.artifacts
                .put(
                    &format!("projects/{}/ingest/{name}", project.id.0),
                    Bytes::from(source),
                    "text/markdown",
                )
                .await
                .expect("source artifact"),
        );
    }
    let first = execute_stage(&app, &project, StageId::Summarize)
        .await
        .expect("schema retry");
    assert_eq!(first.metadata["cache_hit"], false);
    assert_eq!(first.metadata["input_tokens"], 200);
    assert_eq!(first.metadata["cost_usd"], 0.002);
    apply_stage_output(&mut project, StageId::Summarize, first).expect("apply output");
    handle.stop(true).await;
    server_task
        .await
        .expect("server task")
        .expect("server result");
    let second = execute_stage(&app, &project, StageId::Summarize)
        .await
        .expect("cached extract without gateway");
    assert_eq!(second.metadata["cache_hit"], true);
    assert_eq!(second.metadata["input_tokens"], 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    apply_stage_output(&mut project, StageId::Summarize, second).expect("merge repeated artifacts");
    assert_eq!(
        project
            .accumulated_artifacts
            .iter()
            .filter(|artifact| artifact.name == "project_spec.json")
            .count(),
        1
    );
    std::fs::remove_dir_all(root).expect("remove owned fixture");
}
