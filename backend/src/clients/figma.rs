use crate::error::{AutoForgeError, Result};
use bytes::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const FIGMA_API_BASE: &str = "https://api.figma.com/v1";
const MAX_EXPORT_FRAMES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigmaFileRef {
    pub file_key: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigmaScreenExport {
    pub node_id: String,
    pub name: String,
    pub image_bytes: Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigmaDesignExport {
    pub file_key: String,
    pub file_name: Option<String>,
    pub figma_url: String,
    pub screens: Vec<FigmaScreenExport>,
    pub design_json: serde_json::Value,
}

#[derive(Clone)]
pub struct FigmaClient {
    http: Client,
    token: String,
}

#[derive(Debug, Deserialize)]
struct FigmaFileResponse {
    name: Option<String>,
    document: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct FigmaNodesResponse {
    name: Option<String>,
    nodes: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct FigmaImagesResponse {
    images: serde_json::Map<String, Option<String>>,
}

impl FigmaClient {
    pub fn new(token: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| AutoForgeError::FigmaApi(e.to_string()))?;

        Ok(Self {
            http,
            token: token.into(),
        })
    }

    pub fn configured(&self) -> bool {
        !self.token.trim().is_empty()
    }

    pub async fn health_check(&self) -> std::result::Result<(), String> {
        if !self.configured() {
            return Err("FIGMA_ACCESS_TOKEN is not configured".into());
        }

        self.get("/me").await.map(|_| ()).map_err(|e| e.to_string())
    }

    /// Figma 디자인/파일 URL에서 file key와 node id를 추출한다.
    pub fn parse_file_url(url: &str) -> Result<FigmaFileRef> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(AutoForgeError::BadRequest(
                "figma_file_url is required when design_source is figma".into(),
            ));
        }

        let without_query = trimmed.split('?').next().unwrap_or(trimmed);
        let segments: Vec<&str> = without_query
            .trim_end_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();

        let file_key = segments
            .windows(2)
            .find(|window| window[0] == "design" || window[0] == "file" || window[0] == "proto")
            .map(|window| window[1].to_string())
            .ok_or_else(|| {
                AutoForgeError::BadRequest(format!("invalid Figma file URL: {trimmed}"))
            })?;

        let node_id = trimmed
            .split('?')
            .nth(1)
            .and_then(|query| {
                query.split('&').find_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    let key = parts.next()?;
                    if key != "node-id" {
                        return None;
                    }
                    let value = parts.next()?;
                    Some(value.replace('-', ":"))
                })
            })
            .filter(|id| !id.is_empty());

        Ok(FigmaFileRef { file_key, node_id })
    }

    pub async fn export_design(&self, figma_url: &str) -> Result<FigmaDesignExport> {
        if !self.configured() {
            return Err(AutoForgeError::FigmaApi(
                "FIGMA_ACCESS_TOKEN is not configured".into(),
            ));
        }

        let file = Self::parse_file_url(figma_url)?;
        let (file_name, nodes) = if let Some(node_id) = &file.node_id {
            let response: FigmaNodesResponse = self
                .get(&format!("/files/{}/nodes?ids={}", file.file_key, node_id))
                .await?;
            let nodes = collect_nodes_from_map(&response.nodes, Some(node_id))?;
            (response.name, nodes)
        } else {
            let response: FigmaFileResponse =
                self.get(&format!("/files/{}", file.file_key)).await?;
            let nodes = collect_nodes_from_document(&response.document)?;
            (response.name, nodes)
        };

        if nodes.is_empty() {
            return Err(AutoForgeError::FigmaApi(
                "no exportable Figma frames found — open the file and pass a frame URL with node-id"
                    .into(),
            ));
        }

        let node_ids = nodes
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let images: FigmaImagesResponse = self
            .get(&format!(
                "/images/{}?ids={}&format=png&scale=2",
                file.file_key, node_ids
            ))
            .await?;

        let mut screens = Vec::new();
        for (node_id, name) in nodes {
            let image_url = images
                .images
                .get(&node_id)
                .and_then(|value| value.clone())
                .ok_or_else(|| {
                    AutoForgeError::FigmaApi(format!("Figma did not return an image for {node_id}"))
                })?;

            let image_bytes = self.download_image(&image_url).await?;
            screens.push(FigmaScreenExport {
                node_id: node_id.clone(),
                name,
                image_bytes,
            });
        }

        let design_json = serde_json::json!({
            "source": "figma",
            "file_key": file.file_key,
            "file_name": file_name,
            "figma_url": figma_url,
            "screens": screens.iter().map(|screen| serde_json::json!({
                "node_id": screen.node_id,
                "name": screen.name,
            })).collect::<Vec<_>>(),
        });

        Ok(FigmaDesignExport {
            file_key: file.file_key,
            file_name,
            figma_url: figma_url.to_string(),
            screens,
            design_json,
        })
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let url = format!("{FIGMA_API_BASE}{path}");
        let response = self
            .http
            .get(url)
            .header("X-Figma-Token", &self.token)
            .send()
            .await
            .map_err(|e| AutoForgeError::FigmaApi(e.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AutoForgeError::FigmaApi(e.to_string()))?;

        if !status.is_success() {
            return Err(AutoForgeError::FigmaApi(format!(
                "Figma API {status}: {body}"
            )));
        }

        serde_json::from_str(&body).map_err(|e| AutoForgeError::FigmaApi(e.to_string()))
    }

    async fn download_image(&self, url: &str) -> Result<Bytes> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| AutoForgeError::FigmaApi(e.to_string()))?;

        if !response.status().is_success() {
            return Err(AutoForgeError::FigmaApi(format!(
                "failed to download Figma image ({})",
                response.status()
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| AutoForgeError::FigmaApi(e.to_string()))
    }
}

fn collect_nodes_from_document(document: &serde_json::Value) -> Result<Vec<(String, String)>> {
    let mut collected = Vec::new();
    walk_nodes(document, &mut collected);
    if collected.is_empty() {
        return Err(AutoForgeError::FigmaApi(
            "no FRAME/COMPONENT nodes found in Figma file".into(),
        ));
    }
    Ok(collected)
}

fn collect_nodes_from_map(
    nodes: &serde_json::Map<String, serde_json::Value>,
    preferred_node_id: Option<&str>,
) -> Result<Vec<(String, String)>> {
    if let Some(node_id) = preferred_node_id {
        if let Some(node) = nodes.get(node_id) {
            let name = node
                .get("document")
                .and_then(|doc| doc.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("screen")
                .to_string();
            return Ok(vec![(node_id.to_string(), name)]);
        }
    }

    let mut collected = Vec::new();
    for node in nodes.values() {
        if let Some(document) = node.get("document") {
            walk_nodes(document, &mut collected);
        }
    }

    if collected.is_empty() {
        return Err(AutoForgeError::FigmaApi(
            "requested Figma node could not be exported".into(),
        ));
    }

    Ok(collected)
}

fn walk_nodes(node: &serde_json::Value, collected: &mut Vec<(String, String)>) {
    if collected.len() >= MAX_EXPORT_FRAMES {
        return;
    }

    if let (Some(id), Some(name), Some(node_type)) = (
        node.get("id").and_then(|v| v.as_str()),
        node.get("name").and_then(|v| v.as_str()),
        node.get("type").and_then(|v| v.as_str()),
    ) {
        if matches!(node_type, "FRAME" | "COMPONENT" | "SECTION") {
            collected.push((id.to_string(), name.to_string()));
        }
    }

    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for child in children {
            walk_nodes(child, collected);
            if collected.len() >= MAX_EXPORT_FRAMES {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_design_url_with_node_id() {
        let parsed = FigmaClient::parse_file_url(
            "https://www.figma.com/design/AbC12345/My-App?node-id=12-34",
        )
        .unwrap();
        assert_eq!(parsed.file_key, "AbC12345");
        assert_eq!(parsed.node_id.as_deref(), Some("12:34"));
    }

    #[test]
    fn parses_file_url_without_node_id() {
        let parsed =
            FigmaClient::parse_file_url("https://www.figma.com/file/XyZ99999/Legacy-File").unwrap();
        assert_eq!(parsed.file_key, "XyZ99999");
        assert!(parsed.node_id.is_none());
    }
}
