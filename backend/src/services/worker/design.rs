use super::{read_named_text, StageContext, StageExecutor, StageOutput};
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
        let spec: crate::services::ai::ProjectSpec =
            crate::services::ai::parse_json(&read_named_text(ctx, "project_spec.json").await?)?;
        if spec.ui_requirements.is_empty() {
            return Ok(StageOutput {
                artifacts: vec![],
                metadata: serde_json::json!({"skipped": true, "reason": "no UI requirements"}),
            });
        }
        if ctx.model_config.uses_figma_design() {
            return execute_figma(ctx).await;
        }
        execute_stitch(ctx, &spec.ui_requirements).await
    }
}

async fn execute_stitch(ctx: &StageContext, ui_requirements: &[String]) -> Result<StageOutput> {
    let prompt = build_prompt(ui_requirements)?;
    ctx.token_policy.check_input("Stitch UI design", &prompt)?;
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

fn build_prompt(ui_requirements: &[String]) -> Result<String> {
    let requirements = serde_json::to_string(ui_requirements)
        .map_err(|error| AutoForgeError::Internal(error.to_string()))?;
    Ok(format!("Design the requested UI. Preserve explicit accessibility, style and design-token requirements. Do not invent screens or features. ui_requirements: {requirements}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stitch_prompt_contains_only_ui_requirements() {
        let prompt = build_prompt(&["Accessible dark theme login screen".into()]).expect("prompt");
        assert!(prompt.contains("Accessible dark theme"));
        assert!(!prompt.contains("plan.pdf"));
        assert!(!prompt.contains("raw_text.md"));
    }
}
