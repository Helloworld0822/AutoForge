use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const STITCH_MCP_BASE: &str = "https://stitch.googleapis.com/mcp";

#[derive(Clone)]
pub struct StitchClient {
    http: Client,
    api_key: String,
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
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        Ok(Self {
            http,
            api_key: api_key.into(),
        })
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

        let resp = self
            .http
            .post(STITCH_MCP_BASE)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

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
            return Err(AutoForgeError::StitchApi(err.message));
        }

        mcp.result
            .ok_or_else(|| AutoForgeError::StitchApi("empty MCP response".into()))
    }

    pub async fn generate_screen(&self, prompt: &str, device_type: &str) -> Result<StitchScreen> {
        let result = self
            .call_tool(
                "generate_screen",
                serde_json::json!({
                    "prompt": prompt,
                    "deviceType": device_type,
                }),
            )
            .await?;

        serde_json::from_value(result).map_err(|e| AutoForgeError::StitchApi(e.to_string()))
    }

    pub async fn get_screen_html(&self, screen_id: &str) -> Result<StitchAsset> {
        let result = self
            .call_tool(
                "get_screen_html",
                serde_json::json!({ "screenId": screen_id }),
            )
            .await?;

        serde_json::from_value(result).map_err(|e| AutoForgeError::StitchApi(e.to_string()))
    }

    /// Stitch MCP 엔드포인트 연결 확인
    pub async fn health_check(&self) -> std::result::Result<(), String> {
        let body = McpRequest {
            jsonrpc: "2.0",
            id: 0,
            method: "tools/list",
            params: serde_json::json!({}),
        };

        let resp = self
            .http
            .post(STITCH_MCP_BASE)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(15))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("HTTP {status}: {body}"))
        }
    }
}
