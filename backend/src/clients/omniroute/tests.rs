use super::*;
use chrono::{DateTime, Utc};
use std::time::Duration;

#[path = "tests/wire.rs"]
mod wire;

fn request() -> AiRequest {
    AiRequest {
        model: "local-test-model".into(),
        messages: vec![AiMessage {
            role: AiRole::User,
            content: "hello".into(),
        }],
        temperature: Some(0.1),
        max_tokens: Some(100),
        response_format: Some(ResponseFormat {
            kind: "json_object".into(),
        }),
    }
}

fn test_client(base_url: String, max_retries: u8, timeout: Duration) -> OmniRouteClient {
    OmniRouteClient {
        http: Client::builder()
            .timeout(timeout)
            .build()
            .expect("build test client"),
        api_key: "test-key".into(),
        base_url: format!("{base_url}/v1"),
        max_retries,
    }
}

fn valid_completion(content: &str) -> String {
    format!(r#"{{"model":"local-test-model","choices":[{{"message":{{"content":{content:?}}}}}]}}"#)
}

#[tokio::test]
async fn completion_uses_gateway_wire_contract() {
    let (base_url, captured, server) = wire::server(vec![wire::response(
        "200 OK",
        &valid_completion("ok"),
        None,
    )])
    .await;
    let client = test_client(base_url, 0, Duration::from_secs(1));

    let result = client
        .complete(request())
        .await
        .expect("completion succeeds");

    server.await.expect("server completes");
    let captured = captured.lock().expect("capture lock");
    let request = captured.first().expect("one request captured");
    assert_eq!(result.content, "ok");
    assert!(request
        .head
        .starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert_eq!(
        wire::header(request, "authorization"),
        Some("Bearer test-key")
    );
    assert_eq!(wire::header(request, "http-referer"), None);
    assert_eq!(wire::header(request, "x-title"), None);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&request.body).expect("request JSON"),
        serde_json::json!({
            "model": "local-test-model",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.1,
            "max_tokens": 100,
            "response_format": {"type": "json_object"}
        })
    );
}

#[tokio::test]
async fn model_listing_uses_gateway_wire_contract() {
    let (base_url, captured, server) = wire::server(vec![wire::response(
        "200 OK",
        r#"{"data":[{"id":"local-model-alpha","name":"Local model"}]}"#,
        None,
    )])
    .await;
    let client = test_client(base_url, 0, Duration::from_secs(1));

    let models = client.list_models().await.expect("models response parses");

    server.await.expect("server completes");
    let captured = captured.lock().expect("capture lock");
    let request = captured.first().expect("one request captured");
    assert_eq!(models[0].id, "local-model-alpha");
    assert!(request.head.starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert_eq!(
        wire::header(request, "authorization"),
        Some("Bearer test-key")
    );
}

#[tokio::test]
async fn model_listing_status_error_does_not_expose_remote_body() {
    let remote_secret = "remote prompt and credential material";
    let (base_url, _, server) = wire::server(vec![wire::response(
        "401 Unauthorized",
        remote_secret,
        None,
    )])
    .await;
    let client = test_client(base_url, 0, Duration::from_secs(1));

    let error = client.list_models().await.expect_err("status fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("401"));
    assert!(!error.to_string().contains(remote_secret));
}

#[tokio::test]
async fn model_listing_rejects_invalid_json() {
    let (base_url, _, server) =
        wire::server(vec![wire::response("200 OK", "not-json", None)]).await;
    let client = test_client(base_url, 0, Duration::from_secs(1));

    let error = client.list_models().await.expect_err("invalid JSON fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("not valid JSON"));
}

#[tokio::test]
async fn completion_rejects_invalid_json() {
    let (base_url, _, server) =
        wire::server(vec![wire::response("200 OK", "not-json", None)]).await;
    let client = test_client(base_url, 0, Duration::from_secs(1));

    let error = client
        .complete(request())
        .await
        .expect_err("invalid JSON fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("not valid JSON"));
}

#[test]
fn rejects_empty_content() {
    let response = serde_json::from_value::<OmniRouteResponse>(serde_json::json!({
        "choices": [{"message": {"content": null}}]
    }))
    .expect("fixture is valid");

    let error = OmniRouteClient::parse_response(response).expect_err("empty content fails");

    assert!(error.to_string().contains("no content"));
}

#[test]
fn rejects_blank_content() {
    let response = serde_json::from_value::<OmniRouteResponse>(serde_json::json!({
        "choices": [{"message": {"content": " \n\t "}}]
    }))
    .expect("fixture is valid");

    let error = OmniRouteClient::parse_response(response).expect_err("blank content fails");

    assert!(error.to_string().contains("no content"));
}

#[test]
fn rejects_length_finished_response() {
    let response = serde_json::from_value::<OmniRouteResponse>(serde_json::json!({
        "choices": [{
            "finish_reason": "length",
            "message": {"content": "{\"partial\": true"}
        }]
    }))
    .expect("fixture is valid");

    let error = OmniRouteClient::parse_response(response).expect_err("truncated response fails");

    assert!(error.to_string().contains("truncated"));
}

#[tokio::test]
async fn retries_rate_limits_only_until_success() {
    let (base_url, captured, server) = wire::server(vec![
        wire::response("429 Too Many Requests", "ignored", Some("0")),
        wire::response("200 OK", &valid_completion("ok"), None),
    ])
    .await;
    let client = test_client(base_url, 1, Duration::from_secs(1));

    let result = client.complete(request()).await.expect("retry succeeds");

    server.await.expect("server completes");
    assert_eq!(result.content, "ok");
    assert_eq!(captured.lock().expect("capture lock").len(), 2);
}

#[tokio::test]
async fn stops_after_configured_server_error_retries() {
    let remote_secret = "remote prompt and credential material";
    let (base_url, captured, server) = wire::server(vec![
        wire::response("500 Internal Server Error", remote_secret, Some("0")),
        wire::response("503 Service Unavailable", remote_secret, None),
    ])
    .await;
    let client = test_client(base_url, 1, Duration::from_secs(1));

    let error = client
        .complete(request())
        .await
        .expect_err("server errors fail");

    server.await.expect("server completes");
    assert!(error.to_string().contains("503"));
    assert!(!error.to_string().contains(remote_secret));
    assert_eq!(captured.lock().expect("capture lock").len(), 2);
}

#[tokio::test]
async fn does_not_retry_before_an_excessive_retry_after() {
    let (base_url, captured, server) = wire::server(vec![wire::response(
        "429 Too Many Requests",
        "ignored",
        Some("31"),
    )])
    .await;
    let client = test_client(base_url, 3, Duration::from_secs(1));

    let error = client
        .complete(request())
        .await
        .expect_err("excessive wait fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("exceeds the maximum wait"));
    assert_eq!(captured.lock().expect("capture lock").len(), 1);
}

#[tokio::test]
async fn does_not_retry_a_timed_out_post() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept test request");
        let mut request_bytes = [0_u8; 4096];
        let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut request_bytes).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    });
    let client = test_client(format!("http://{address}"), 3, Duration::from_millis(20));

    let error = client.complete(request()).await.expect_err("timeout fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("timed out"));
}

#[tokio::test]
async fn does_not_retry_an_ambiguous_post_network_failure() {
    let (base_url, captured, server) = wire::close_after_request_server().await;
    let client = test_client(base_url, 3, Duration::from_secs(1));

    let error = client
        .complete(request())
        .await
        .expect_err("connection close fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("HTTP request failed"));
    assert_eq!(captured.lock().expect("capture lock").len(), 1);
}

#[tokio::test]
async fn rejects_response_bodies_over_the_bound() {
    let (base_url, _, server) = wire::server(vec![wire::response_with_content_length(
        "200 OK",
        MAX_RESPONSE_BODY_BYTES + 1,
    )])
    .await;
    let client = test_client(base_url, 0, Duration::from_secs(1));

    let error = client
        .complete(request())
        .await
        .expect_err("oversized body fails");

    server.await.expect("server completes");
    assert!(error.to_string().contains("maximum size"));
}

#[test]
fn retry_after_seconds_and_dates_respect_the_bound() {
    let now = DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
        .expect("valid timestamp")
        .with_timezone(&Utc);
    let seconds = reqwest::header::HeaderValue::from_static("30");
    let date = reqwest::header::HeaderValue::from_static("Tue, 01 Jan 2030 00:00:30 GMT");
    let excessive = reqwest::header::HeaderValue::from_static("31");

    assert_eq!(
        OmniRouteClient::retry_delay(0, Some(&seconds), now),
        RetryDelay::Wait(Duration::from_secs(30))
    );
    assert_eq!(
        OmniRouteClient::retry_delay(0, Some(&date), now),
        RetryDelay::Wait(Duration::from_secs(30))
    );
    assert_eq!(
        OmniRouteClient::retry_delay(0, Some(&excessive), now),
        RetryDelay::ExceedsMaximum
    );
}
