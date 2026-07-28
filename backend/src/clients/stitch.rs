use crate::clients::stitch_token::StitchTokenProvider;
use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const STITCH_MCP_BASE: &str = "https://stitch.googleapis.com/mcp";

#[derive(Clone)]
pub struct StitchClient {
    http: Client,
    api_key: String,
    bearer: StitchTokenProvider,
    google_cloud_project: Option<String>,
}

#[derive(Debug, Serialize)]
struct McpRequest {
    jsonrpc: &'static str,
    id: u32,
    method: &'static str,
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct McpResponse {
    result: Option<serde_json::Value>,
    error: Option<McpError>,
}

#[derive(Debug, Deserialize)]
struct McpError {
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StitchScreen {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StitchAsset {
    pub download_url: String,
}

impl StitchClient {
    pub fn new(
        api_key: impl Into<String>,
        access_token: impl Into<String>,
        google_cloud_project: Option<String>,
    ) -> Result<Self> {
        Self::with_bearer(
            api_key,
            StitchTokenProvider::from_env(access_token.into(), google_cloud_project.clone()),
            google_cloud_project,
        )
    }

    #[cfg(test)]
    fn new_for_test(
        api_key: impl Into<String>,
        access_token: impl Into<String>,
        google_cloud_project: Option<String>,
    ) -> Result<Self> {
        let token = access_token.into();
        let bearer = if token.is_empty() {
            StitchTokenProvider::static_only(None)
        } else {
            StitchTokenProvider::static_only(Some(token))
        };
        Self::with_bearer(api_key, bearer, google_cloud_project)
    }

    fn with_bearer(
        api_key: impl Into<String>,
        bearer: StitchTokenProvider,
        google_cloud_project: Option<String>,
    ) -> Result<Self> {
        let google_cloud_project =
            google_cloud_project.or_else(|| bearer.quota_project().map(str::to_string));

        let http = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        Ok(Self {
            http,
            api_key: api_key.into(),
            bearer,
            google_cloud_project,
        })
    }

    fn has_credentials(&self) -> bool {
        !self.api_key.is_empty() || self.bearer.can_provide_token()
    }

    /// Stitch MCP는 API 키와 Bearer를 동시에내면 401을 반환한다.
    /// AI 생성 도구는 Bearer만, 그 외는 API 키만 사용한다 (없으면 Bearer로 폴백).
    fn auth_headers_for_call(
        &self,
        method: &str,
        tool_name: Option<&str>,
        bearer_token: Option<&str>,
    ) -> Vec<(&'static str, String)> {
        let mut headers = Vec::new();
        let use_bearer = self.needs_access_token(method, tool_name);

        if use_bearer {
            if let Some(token) = bearer_token {
                headers.push(("Authorization", format!("Bearer {token}")));
                if let Some(project) = &self.google_cloud_project {
                    headers.push(("X-Goog-User-Project", project.clone()));
                }
            }
        } else if !self.api_key.is_empty() {
            headers.push(("X-Goog-Api-Key", self.api_key.clone()));
        } else if let Some(token) = bearer_token {
            headers.push(("Authorization", format!("Bearer {token}")));
            if let Some(project) = &self.google_cloud_project {
                headers.push(("X-Goog-User-Project", project.clone()));
            }
        }

        headers
    }

    fn needs_access_token(&self, method: &str, tool_name: Option<&str>) -> bool {
        method == "tools/call"
            && tool_name.is_some_and(|name| {
                matches!(
                    name,
                    "generate_screen_from_text" | "edit_screens" | "generate_variants"
                )
            })
    }

    async fn post_mcp(&self, body: &McpRequest) -> Result<serde_json::Value> {
        if !self.has_credentials() {
            return Err(AutoForgeError::StitchApi(
                "STITCH_API_KEY or Stitch Bearer credentials are not configured".into(),
            ));
        }

        let tool_name = body.params.get("name").and_then(|v| v.as_str());
        let needs_bearer = self.needs_access_token(body.method, tool_name);

        if needs_bearer && !self.bearer.can_provide_token() {
            return Err(AutoForgeError::StitchApi(
                "Stitch Bearer token is required for screen generation — set STITCH_ACCESS_TOKEN, \
                 mount GOOGLE_APPLICATION_CREDENTIALS, or run \
                 `gcloud auth application-default login`"
                    .into(),
            ));
        }

        self.post_mcp_with_retry(body, tool_name, needs_bearer, false)
            .await
    }

    async fn post_mcp_with_retry(
        &self,
        body: &McpRequest,
        tool_name: Option<&str>,
        needs_bearer: bool,
        force_refresh: bool,
    ) -> Result<serde_json::Value> {
        let bearer_token = if needs_bearer || self.api_key.is_empty() {
            Some(self.bearer.access_token(force_refresh).await?)
        } else {
            None
        };

        let mut req = self
            .http
            .post(STITCH_MCP_BASE)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(body);

        for (name, value) in
            self.auth_headers_for_call(body.method, tool_name, bearer_token.as_deref())
        {
            req = req.header(name, value);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && needs_bearer
            && !force_refresh
            && self.bearer.can_provide_token()
        {
            tracing::info!("Stitch Bearer token rejected (401); refreshing and retrying once");
            return Box::pin(self.post_mcp_with_retry(body, tool_name, needs_bearer, true)).await;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::StitchApi(format!(
                "MCP call failed ({status}): {body}"
            )));
        }

        let mcp: McpResponse = resp
            .json()
            .await
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        if let Some(err) = mcp.error {
            let tool = body
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>");
            return Err(AutoForgeError::StitchApi(format!(
                "{} (tool: {tool})",
                err.message
            )));
        }

        let result = mcp
            .result
            .ok_or_else(|| AutoForgeError::StitchApi("empty MCP response".into()))?;

        if result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let message =
                extract_text_content(&result).unwrap_or_else(|| "unknown Stitch tool error".into());
            if needs_bearer
                && !force_refresh
                && message.contains("authentication")
                && self.bearer.can_provide_token()
            {
                tracing::info!("Stitch tool auth error; refreshing Bearer token and retrying once");
                return Box::pin(self.post_mcp_with_retry(body, tool_name, needs_bearer, true))
                    .await;
            }
            return Err(AutoForgeError::StitchApi(message));
        }

        Ok(result)
    }

    async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let body = McpRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "tools/call",
            params: serde_json::json!({
                "name": tool_name,
                "arguments": arguments,
            }),
        };

        self.post_mcp(&body).await
    }

    /// Stitch 프로젝트가 없으면 생성하고 ID를 반환한다.
    pub async fn ensure_project(&self, title: &str, existing_id: Option<&str>) -> Result<String> {
        if let Some(id) = existing_id.filter(|id| !id.is_empty()) {
            return Ok(id.to_string());
        }

        let result = self
            .call_tool("create_project", serde_json::json!({ "title": title }))
            .await?;

        extract_project_id(&result)
    }

    pub async fn generate_screen(
        &self,
        project_id: &str,
        prompt: &str,
        device_type: &str,
    ) -> Result<StitchScreen> {
        let result = self
            .call_tool(
                "generate_screen_from_text",
                serde_json::json!({
                    "projectId": project_id,
                    "prompt": prompt,
                    "deviceType": device_type,
                }),
            )
            .await?;

        extract_screen(&result)
    }

    pub async fn get_screen_html(&self, project_id: &str, screen_id: &str) -> Result<StitchAsset> {
        let name = format!("projects/{project_id}/screens/{screen_id}");
        let result = self
            .call_tool(
                "get_screen",
                serde_json::json!({
                    "name": name,
                    "projectId": project_id,
                    "screenId": screen_id,
                }),
            )
            .await?;

        let download_url = find_download_url(&result).ok_or_else(|| {
            AutoForgeError::StitchApi("no HTML download URL in get_screen response".into())
        })?;

        Ok(StitchAsset { download_url })
    }

    /// Stitch MCP 엔드포인트 연결 확인 (tools/list + 생성 인증 여부)
    pub async fn health_check(&self) -> std::result::Result<(), String> {
        if !self.has_credentials() {
            return Err("STITCH_API_KEY or Stitch Bearer credentials not configured".into());
        }

        let body = McpRequest {
            jsonrpc: "2.0",
            id: 0,
            method: "tools/list",
            params: serde_json::json!({}),
        };

        self.post_mcp(&body).await.map_err(|e| e.to_string())?;

        if !self.bearer.can_provide_token() {
            return Err(
                "tools/list OK but Stitch Bearer credentials missing — Design stage will fail on generate_screen"
                    .into(),
            );
        }

        self.bearer
            .access_token(false)
            .await
            .map_err(|e| format!("Stitch Bearer token refresh failed: {e}"))?;

        Ok(())
    }
}

fn extract_text_content(value: &serde_json::Value) -> Option<String> {
    value
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
}

fn parse_json_text(value: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(text) = extract_text_content(value) {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
            return Some(parsed);
        }
    }
    Some(value.clone())
}

fn extract_project_id(value: &serde_json::Value) -> Result<String> {
    let parsed = parse_json_text(value).unwrap_or_else(|| value.clone());

    if let Some(id) = parsed
        .get("name")
        .and_then(|n| n.as_str())
        .and_then(|n| n.strip_prefix("projects/"))
    {
        return Ok(id.to_string());
    }

    if let Some(id) = parsed.get("projectId").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }

    if let Some(id) = parsed.get("id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }

    if let Some(id) = find_string_field(&parsed, "projectId") {
        return Ok(id);
    }

    Err(AutoForgeError::StitchApi(format!(
        "could not parse project id from Stitch response: {parsed}"
    )))
}

fn extract_screen(value: &serde_json::Value) -> Result<StitchScreen> {
    let parsed = parse_json_text(value).unwrap_or_else(|| value.clone());

    if let Some(screen) = find_screen_object(&parsed) {
        return Ok(screen);
    }

    Err(AutoForgeError::StitchApi(format!(
        "could not parse screen from Stitch response: {parsed}"
    )))
}

fn find_screen_object(value: &serde_json::Value) -> Option<StitchScreen> {
    if let Some(name) = value.get("name").and_then(|n| n.as_str()) {
        if let Some(screen_id) = name.rsplit('/').next() {
            return Some(StitchScreen {
                id: screen_id.to_string(),
                name: Some(name.to_string()),
            });
        }
    }

    if let Some(screen_id) = value.get("screenId").and_then(|v| v.as_str()) {
        return Some(StitchScreen {
            id: screen_id.to_string(),
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }

    for key in ["screens", "items", "data", "result"] {
        if let Some(arr) = value.get(key).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(screen) = find_screen_object(item) {
                    return Some(screen);
                }
            }
        }
    }

    if let Some(obj) = value.as_object() {
        for child in obj.values() {
            if let Some(screen) = find_screen_object(child) {
                return Some(screen);
            }
        }
    }

    None
}

fn find_download_url(value: &serde_json::Value) -> Option<String> {
    if let Some(url) = value.get("downloadUrl").and_then(|v| v.as_str()) {
        return Some(url.to_string());
    }

    if let Some(html) = value.get("htmlCode") {
        if let Some(url) = html.get("downloadUrl").and_then(|v| v.as_str()) {
            return Some(url.to_string());
        }
    }

    match value {
        serde_json::Value::Object(map) => {
            for child in map.values() {
                if let Some(url) = find_download_url(child) {
                    return Some(url);
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(url) = find_download_url(item) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_string_field(value: &serde_json::Value, field: &str) -> Option<String> {
    if let Some(s) = value.get(field).and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }

    match value {
        serde_json::Value::Object(map) => {
            for child in map.values() {
                if let Some(s) = find_string_field(child, field) {
                    return Some(s);
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(s) = find_string_field(item, field) {
                    return Some(s);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_project_id_from_name() {
        let value = serde_json::json!({
            "content": [{
                "type": "text",
                "text": "{\"name\":\"projects/4044680601076201931\"}"
            }]
        });
        assert_eq!(extract_project_id(&value).unwrap(), "4044680601076201931");
    }

    #[test]
    fn parses_screen_from_name() {
        let value = serde_json::json!({
            "screens": [{
                "name": "projects/123/screens/abc456"
            }]
        });
        let screen = extract_screen(&value).unwrap();
        assert_eq!(screen.id, "abc456");
        assert_eq!(screen.name.as_deref(), Some("projects/123/screens/abc456"));
    }

    #[test]
    fn finds_html_download_url() {
        let value = serde_json::json!({
            "htmlCode": { "downloadUrl": "https://example.com/screen.html" }
        });
        assert_eq!(
            find_download_url(&value).as_deref(),
            Some("https://example.com/screen.html")
        );
    }

    #[test]
    fn bearer_only_for_generate_screen_tool() {
        let client = StitchClient::new_for_test("api-key", "access-token", None).unwrap();
        let headers = client.auth_headers_for_call(
            "tools/call",
            Some("generate_screen_from_text"),
            Some("access-token"),
        );
        let names: Vec<_> = headers.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["Authorization"]);
        assert!(!names.contains(&"X-Goog-Api-Key"));
    }

    #[test]
    fn api_key_only_for_create_project() {
        let client = StitchClient::new_for_test("api-key", "access-token", None).unwrap();
        let headers = client.auth_headers_for_call(
            "tools/call",
            Some("create_project"),
            Some("access-token"),
        );
        let names: Vec<_> = headers.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["X-Goog-Api-Key"]);
        assert!(!names.contains(&"Authorization"));
    }

    #[tokio::test]
    async fn health_check_fails_without_any_stitch_credential() {
        let client = StitchClient::new_for_test("", "", None).unwrap();
        let err = client.health_check().await.unwrap_err();
        assert!(err.contains("not configured"), "{err}");
    }

    #[tokio::test]
    async fn stitch_calls_fail_without_any_credential() {
        let client = StitchClient::new_for_test("", "", None).unwrap();
        let err = client.ensure_project("title", None).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not configured"), "{msg}");
    }

    #[tokio::test]
    async fn generate_screen_requires_bearer_credentials_when_only_api_key() {
        let client = StitchClient::new_for_test("test-api-key", "", None).unwrap();
        let err = client
            .generate_screen("proj-id", "build a login page", "MOBILE")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Bearer"), "{msg}");
    }
}
