use super::{StageContext, StageExecutor, StageOutput};
use crate::domain::{ArtifactRef, StageId};
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use bytes::Bytes;

pub struct DesignExecutor;

#[async_trait]
impl StageExecutor for DesignExecutor {
    fn stage(&self) -> StageId {
        StageId::Design
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        if ctx.model_config.uses_figma_design() {
            return execute_figma(ctx).await;
        }
        execute_stitch(ctx).await
    }
}

async fn execute_stitch(ctx: &StageContext) -> Result<StageOutput> {
    let prompt = build_prompt(&ctx.input);
    let device_type = ctx.model_config.design_device_type();
    let existing_project = ctx
        .stage_outputs
        .get(&StageId::Design)
        .and_then(|value| value.get("stitch_project_id"))
        .and_then(serde_json::Value::as_str);
    let project_title = format!("AutoForge {}", ctx.command.project_id.0);
    let project_id = ctx
        .stitch
        .ensure_project(&project_title, existing_project)
        .await?;
    let screen = ctx
        .stitch
        .generate_screen(&project_id, &prompt, device_type)
        .await?;
    let html = ctx.stitch.get_screen_html(&project_id, &screen.id).await?;

    Ok(StageOutput {
        artifacts: vec![ArtifactRef {
            name: format!("screens/{}.html", screen.id),
            key: html.download_url.clone(),
            uri: html.download_url,
            content_type: "text/html".into(),
            sha256: None,
        }],
        metadata: serde_json::json!({
            "screen_id": screen.id,
            "screen_name": screen.name,
            "stitch_project_id": project_id,
            "design_source": "stitch",
        }),
    })
}

async fn execute_figma(ctx: &StageContext) -> Result<StageOutput> {
    let figma_url = ctx
        .model_config
        .figma_file_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            AutoForgeError::BadRequest(
                "figma_file_url is required when design_source is figma".into(),
            )
        })?;
    let export = ctx.figma.export_design(figma_url).await?;
    let base = format!("projects/{}/design", ctx.command.project_id.0);
    let mut artifacts = Vec::new();
    let design_json = ctx
        .artifacts
        .put(
            &format!("{base}/figma-design.json"),
            Bytes::from(export.design_json.to_string()),
            "application/json",
        )
        .await?;
    artifacts.push(design_json);

    let mut screens = Vec::new();
    for screen in export.screens {
        let slug = slugify_filename(&screen.name);
        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/screens/{slug}.png"),
                screen.image_bytes,
                "image/png",
            )
            .await?;
        screens.push(serde_json::json!({
            "node_id": screen.node_id,
            "name": screen.name,
            "artifact": artifact.name,
            "uri": artifact.uri,
        }));
        artifacts.push(artifact);
    }

    Ok(StageOutput {
        artifacts,
        metadata: serde_json::json!({
            "design_source": "figma",
            "figma_file_key": export.file_key,
            "figma_file_name": export.file_name,
            "figma_file_url": export.figma_url,
            "screens": screens,
        }),
    })
}

fn slugify_filename(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "screen".into()
    } else {
        trimmed.to_string()
    }
}

fn build_prompt(inputs: &[ArtifactRef]) -> String {
    format!(
        "ui_requirements를 반영한 모던 UI 대시보드를 디자인하세요.\n입력: {:?}",
        inputs
            .iter()
            .map(|artifact| &artifact.uri)
            .collect::<Vec<_>>()
    )
}
