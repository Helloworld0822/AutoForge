use super::*;

#[test]
fn parses_usage_and_cached_tokens() {
    let response = serde_json::from_value::<OmniRouteResponse>(serde_json::json!({
        "model": "test/model", "choices": [{"message": {"content": "{}"}}],
        "usage": {"prompt_tokens": 100, "completion_tokens": 20,
            "prompt_tokens_details": {"cached_tokens": 40}, "cost": 0.12}
    }))
    .expect("fixture is valid");
    let parsed = OmniRouteClient::parse_response(response).expect("response parses");
    assert_eq!(parsed.usage.input_tokens, 100);
    assert_eq!(parsed.usage.output_tokens, 20);
    assert_eq!(parsed.usage.cached_tokens, 40);
    assert_eq!(parsed.usage.cost_usd, Some(0.12));
}

#[test]
fn builds_structured_json_request() {
    let body = OmniRouteClient::request_body(&request());
    assert_eq!(body["response_format"]["type"], "json_object");
    assert_eq!(body["messages"][0]["role"], "user");
}

#[test]
fn missing_cost_is_unknown_and_empty_choices_are_rejected() {
    let response: OmniRouteResponse =
        serde_json::from_str(&valid_completion("{}")).expect("fixture");
    assert_eq!(
        OmniRouteClient::parse_response(response)
            .expect("parse")
            .usage
            .cost_usd,
        None
    );
    let empty: OmniRouteResponse =
        serde_json::from_value(serde_json::json!({"choices":[]})).expect("fixture");
    assert!(OmniRouteClient::parse_response(empty).is_err());
}
