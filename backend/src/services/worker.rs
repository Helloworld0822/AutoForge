use crate::clients::cursor::{CreateAgentOpts, CursorClient};
use crate::clients::figma::FigmaClient;
use crate::clients::model_router::{ModelRole, ModelRouter};
use crate::clients::openrouter::{AiProvider, OpenRouterClient};
use crate::clients::stitch::StitchClient;
use crate::domain::{
    ArtifactRef, LanguageMode, PipelineModelConfig, ProgrammingLanguage, StageCommand, StageId,
};
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{
    complete_json, parse_json, project_spec_system, ProjectSpec, QuestionList,
};
use crate::services::artifacts::ArtifactStore;
use crate::services::ingest::{ingest_devops_plan, ingest_pdf};
use crate::services::language::{language_prompt_note, resolve_effective_language};
use crate::services::quality::{
    DebugReport, SecurityReport, VerifyReport, SECURITY_CHECKS, VERIFY_CHECKS,
};
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;

pub struct StageContext {
    pub command: StageCommand,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub cursor: Arc<CursorClient>,
    pub openrouter: Arc<OpenRouterClient>,
    pub model_router: ModelRouter,
    pub deepseek_debug_retries: u8,
    pub mid_debug_retries: u8,
    pub opus_max_calls: u8,
    pub project_budget_usd: f64,
    pub task_budget_usd: f64,
    pub stitch: Arc<StitchClient>,
    pub figma: Arc<FigmaClient>,
    pub input: Vec<ArtifactRef>,
    pub repo_url: Option<String>,
    pub stage_outputs: HashMap<StageId, serde_json::Value>,
    pub pr_url: Option<String>,
    pub language_mode: LanguageMode,
    pub programming_language: Option<ProgrammingLanguage>,
    pub resolved_language: Option<ProgrammingLanguage>,
    pub architecture_finalize: bool,
    pub architecture_answers: Vec<(String, String)>,
    pub model_config: PipelineModelConfig,
}

#[derive(Debug)]
pub struct StageOutput {
    pub artifacts: Vec<ArtifactRef>,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait StageExecutor: Send + Sync {
    fn stage(&self) -> StageId;
    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput>;
}

async fn read_named_text(ctx: &StageContext, name: &str) -> Result<String> {
    let artifact = ctx
        .input
        .iter()
        .find(|artifact| artifact.name == name)
        .ok_or_else(|| AutoForgeError::Ingest(format!("missing {name} artifact")))?;
    let bytes = ctx.artifacts.get(&artifact.key).await?;
    String::from_utf8(bytes.to_vec())
        .map_err(|error| AutoForgeError::Ingest(format!("{name} is not UTF-8: {error}")))
}

pub struct IngestExecutor;

#[async_trait]
impl StageExecutor for IngestExecutor {
    fn stage(&self) -> StageId {
        StageId::Ingest
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let pdf_ref = ctx
            .input
            .iter()
            .find(|a| a.name.ends_with(".pdf") && !a.name.contains("devops"))
            .or_else(|| ctx.input.iter().find(|a| a.name == "plan.pdf"))
            .ok_or_else(|| AutoForgeError::Ingest("missing PDF input".into()))?;

        let bytes = ctx.artifacts.get(&pdf_ref.key).await?;
        let result = ingest_pdf(&bytes)?;
        let base = format!("projects/{}/ingest", ctx.command.project_id.0);

        let text_uri = ctx
            .artifacts
            .put(
                &format!("{base}/raw_text.md"),
                Bytes::from(result.raw_text.clone()),
                "text/markdown",
            )
            .await?;

        let mut meta = serde_json::json!({
            "page_count": result.page_count,
            "sha256": result.sha256,
            "has_devops_plan": false,
        });

        let mut artifacts = vec![text_uri.clone()];

        // DevOps 계획서 ingest (선택)
        let devops_ref = ctx.input.iter().find(|a| a.name.starts_with("devops_plan"));

        if let Some(devops_ref) = devops_ref {
            let devops_bytes = ctx.artifacts.get(&devops_ref.key).await?;
            let devops_input = crate::domain::DevopsPlanInput {
                filename: Some(devops_ref.name.clone()),
                content_type: Some(devops_ref.content_type.clone()),
                bytes: Some(devops_bytes.to_vec()),
                text: None,
            };
            if let Ok(devops) = ingest_devops_plan(&devops_input) {
                let devops_uri = ctx
                    .artifacts
                    .put(
                        &format!("{base}/devops_raw_text.md"),
                        Bytes::from(devops.raw_text.clone()),
                        "text/markdown",
                    )
                    .await?;
                artifacts.push(devops_uri);
                meta["has_devops_plan"] = serde_json::json!(true);
                meta["devops_format"] = serde_json::json!(devops.format);
                meta["devops_source"] = serde_json::json!(devops.source);
                meta["devops_sha256"] = serde_json::json!(devops.sha256);
            }
        }

        let meta_uri = ctx
            .artifacts
            .put(
                &format!("{base}/ingest_meta.json"),
                Bytes::from(meta.to_string()),
                "application/json",
            )
            .await?;

        artifacts.push(meta_uri);

        Ok(StageOutput {
            artifacts,
            metadata: meta,
        })
    }
}

pub struct SummarizeExecutor;

#[async_trait]
impl StageExecutor for SummarizeExecutor {
    fn stage(&self) -> StageId {
        StageId::Summarize
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let raw_text = read_named_text(ctx, "raw_text.md").await?;
        let response = complete_json(
            &ctx.openrouter,
            ctx.model_router.model(ModelRole::Extract),
            project_spec_system(),
            format!("Extract this source document into project_spec.json:\n\n{raw_text}"),
        )
        .await?;
        let spec: ProjectSpec = parse_json(&response.content)?;
        let text = serde_json::to_string_pretty(&spec)
            .map_err(|error| AutoForgeError::OpenRouter(error.to_string()))?;

        let resolved =
            resolve_effective_language(ctx.language_mode, ctx.programming_language, &text);

        let base = format!("projects/{}/extract", ctx.command.project_id.0);
        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/project_spec.json"),
                Bytes::from(text),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "model": response.model,
                "input_tokens": response.usage.input_tokens,
                "output_tokens": response.usage.output_tokens,
                "cost_usd": response.usage.cost_usd,
                "programming_language": resolved.as_str(),
                "ui_required": !spec.ui_requirements.is_empty(),
            }),
        })
    }
}

pub struct ArchitectExecutor;

#[async_trait]
impl StageExecutor for ArchitectExecutor {
    fn stage(&self) -> StageId {
        StageId::Architect
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        if ctx.architecture_finalize {
            return run_architect_finalize(ctx).await;
        }

        let spec = read_named_text(ctx, "project_spec.json").await?;
        let response = complete_json(
            &ctx.openrouter,
            ctx.model_router.model(ModelRole::Plan),
            "Create clarification questions from the structured project spec. Do not invent requirements.",
            format!("project_spec.json:\n{spec}"),
        )
        .await?;
        let question_list: QuestionList = parse_json(&response.content)?;
        let questions = question_list.questions;

        if questions.is_empty() {
            return run_architect_finalize_with_answers(ctx, &[]).await;
        }

        let base = format!("projects/{}/architect", ctx.command.project_id.0);
        let questions_json = serde_json::to_string(&serde_json::json!({ "questions": questions }))
            .unwrap_or_default();
        let draft = ctx
            .artifacts
            .put(
                &format!("{base}/clarifications.json"),
                Bytes::from(questions_json),
                "application/json",
            )
            .await?;

        let question_views: Vec<_> = questions
            .iter()
            .map(|q| {
                serde_json::json!({
                    "id": q.id,
                    "question": q.question,
                    "options": q.options,
                    "required": q.required,
                    "category": q.category,
                })
            })
            .collect();

        Ok(StageOutput {
            artifacts: vec![draft],
            metadata: serde_json::json!({
                "phase": "draft",
                "model": response.model,
                "questions": question_views,
                "question_count": questions.len(),
            }),
        })
    }
}

pub struct DesignExecutor;

#[async_trait]
impl StageExecutor for DesignExecutor {
    fn stage(&self) -> StageId {
        StageId::Design
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        if ctx.model_config.uses_figma_design() {
            return execute_figma_design(ctx).await;
        }
        execute_stitch_design(ctx).await
    }
}

async fn execute_stitch_design(ctx: &StageContext) -> Result<StageOutput> {
    let prompt = build_design_prompt(&ctx.input);
    let device_type = ctx.model_config.design_device_type();

    let existing_stitch_project = ctx
        .stage_outputs
        .get(&StageId::Design)
        .and_then(|v| v.get("stitch_project_id"))
        .and_then(|v| v.as_str());

    let project_title = format!("AutoForge {}", ctx.command.project_id.0);
    let stitch_project_id = ctx
        .stitch
        .ensure_project(&project_title, existing_stitch_project)
        .await?;

    let screen = ctx
        .stitch
        .generate_screen(&stitch_project_id, &prompt, device_type)
        .await?;
    let html = ctx
        .stitch
        .get_screen_html(&stitch_project_id, &screen.id)
        .await?;

    let artifact = ArtifactRef {
        name: format!("screens/{}.html", screen.id),
        key: html.download_url.clone(),
        uri: html.download_url,
        content_type: "text/html".into(),
        sha256: None,
    };

    Ok(StageOutput {
        artifacts: vec![artifact],
        metadata: serde_json::json!({
            "screen_id": screen.id,
            "screen_name": screen.name,
            "stitch_project_id": stitch_project_id,
            "design_source": "stitch",
        }),
    })
}

async fn execute_figma_design(ctx: &StageContext) -> Result<StageOutput> {
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

    let mut screen_meta = Vec::new();
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
        screen_meta.push(serde_json::json!({
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
            "screens": screen_meta,
        }),
    })
}

fn slugify_filename(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
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

pub struct ImplementExecutor;

#[async_trait]
impl StageExecutor for ImplementExecutor {
    fn stage(&self) -> StageId {
        StageId::Implement
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let repo_url = ctx
            .repo_url
            .as_deref()
            .ok_or_else(|| AutoForgeError::BadRequest("repo_url required".into()))?;

        let prompt = build_implement_prompt(ctx);
        let profile = ctx.model_config.profile_for(StageId::Implement);

        let opts = CreateAgentOpts {
            repo_url: Some(repo_url),
            starting_ref: Some("main"),
            auto_create_pr: Some(true),
            agent_id: None,
        };

        let resp = ctx.cursor.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(15),
            )
            .await?;

        let pr_url = run
            .result
            .and_then(|r| r.git)
            .and_then(|g| g.branches)
            .and_then(|b| b.into_iter().next())
            .and_then(|br| br.pr_url);

        Ok(StageOutput {
            artifacts: vec![],
            metadata: serde_json::json!({
                "cursor_agent_id": resp.agent.id,
                "pr_url": pr_url,
            }),
        })
    }
}

pub struct VerifyExecutor;

#[async_trait]
impl StageExecutor for VerifyExecutor {
    fn stage(&self) -> StageId {
        StageId::Verify
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let repo_url = ctx
            .repo_url
            .as_deref()
            .ok_or_else(|| AutoForgeError::BadRequest("repo_url required for verify".into()))?;

        let prompt = build_verify_prompt(ctx);
        let profile = ctx.model_config.profile_for(StageId::Verify);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref());

        let resp = ctx.cursor.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(15),
            )
            .await?;

        let text = run.result_text().unwrap_or_default();
        let report = VerifyReport::parse_from_agent_text(&text);
        let base = format!("projects/{}/verify", ctx.command.project_id.0);

        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/verify_report.json"),
                Bytes::from(serde_json::to_string(&report).unwrap_or_default()),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "passed": report.passed,
                "errors": report.errors.len(),
                "cursor_agent_id": resp.agent.id,
            }),
        })
    }
}

pub struct DebugExecutor;

#[async_trait]
impl StageExecutor for DebugExecutor {
    fn stage(&self) -> StageId {
        StageId::Debug
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let repo_url = ctx
            .repo_url
            .as_deref()
            .ok_or_else(|| AutoForgeError::BadRequest("repo_url required for debug".into()))?;

        let verify_meta = ctx
            .stage_outputs
            .get(&StageId::Verify)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "passed": false }));

        let role = ctx.model_router.debug_role(
            ctx.command.attempt,
            ctx.deepseek_debug_retries,
            ctx.mid_debug_retries,
        );
        let diagnosis = if ctx.command.attempt >= ctx.deepseek_debug_retries {
            let response = crate::services::ai::complete_json(
                &ctx.openrouter,
                ctx.model_router.model(role),
                "Diagnose the verification failure only. Return root cause, affected files, recommended fix, risk, and additional tests.",
                format!("verify metadata:\n{verify_meta}"),
            )
            .await?;
            Some((response.content, response.model, response.usage))
        } else {
            None
        };
        let prompt = build_debug_prompt(
            ctx,
            &verify_meta,
            diagnosis.as_ref().map(|value| value.0.as_str()),
        );
        let profile = ctx.model_config.profile_for(StageId::Debug);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref());

        let resp = ctx.cursor.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(20),
            )
            .await?;

        let text = run.result_text().unwrap_or_default();
        let report = DebugReport {
            fixes_applied: vec!["auto-debug via Codex".into()],
            files_changed: vec![],
            summary: text.chars().take(300).collect(),
            resolved_errors: 0,
        };

        let base = format!("projects/{}/debug", ctx.command.project_id.0);
        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/debug_report.json"),
                Bytes::from(serde_json::to_string(&report).unwrap_or_default()),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "debug_cycle": ctx.command.attempt,
                "debug_role": format!("{role:?}"),
                "diagnosis_model": diagnosis.as_ref().map(|value| value.1.clone()),
                "diagnosis_usage": diagnosis.as_ref().map(|value| &value.2),
                "cursor_agent_id": resp.agent.id,
            }),
        })
    }
}

pub struct SecurityPatchExecutor;

#[async_trait]
impl StageExecutor for SecurityPatchExecutor {
    fn stage(&self) -> StageId {
        StageId::SecurityPatch
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let repo_url = ctx.repo_url.as_deref().ok_or_else(|| {
            AutoForgeError::BadRequest("repo_url required for security patch".into())
        })?;

        let prompt = build_security_prompt(ctx);
        let profile = ctx.model_config.profile_for(StageId::SecurityPatch);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref());

        let resp = ctx.cursor.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(20),
            )
            .await?;

        let text = run.result_text().unwrap_or_default();
        let report = SecurityReport::parse_from_agent_text(&text);
        let base = format!("projects/{}/security", ctx.command.project_id.0);

        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/security_report.json"),
                Bytes::from(serde_json::to_string(&report).unwrap_or_default()),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "passed": report.passed,
                "vulnerabilities_found": report.vulnerabilities_found,
                "patches_applied": report.patches_applied.len(),
                "cursor_agent_id": resp.agent.id,
            }),
        })
    }
}

pub struct DeliverExecutor;

#[async_trait]
impl StageExecutor for DeliverExecutor {
    fn stage(&self) -> StageId {
        StageId::Deliver
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        let manifest = serde_json::json!({
            "project_id": ctx.command.project_id.0,
            "pr_url": ctx.pr_url,
            "artifacts": ctx.input.iter().map(|a| &a.uri).collect::<Vec<_>>(),
            "stage_outputs": ctx.stage_outputs,
            "delivered_at": chrono::Utc::now().to_rfc3339(),
        });

        let base = format!("projects/{}/deliver", ctx.command.project_id.0);
        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/delivery_manifest.json"),
                Bytes::from(manifest.to_string()),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: manifest,
        })
    }
}

fn agent_opts<'a>(repo_url: &'a str, _pr_url: Option<&'a str>) -> CreateAgentOpts<'a> {
    CreateAgentOpts {
        repo_url: Some(repo_url),
        starting_ref: Some("main"),
        auto_create_pr: Some(false),
        agent_id: None,
    }
}

pub fn executors() -> Vec<Arc<dyn StageExecutor>> {
    vec![
        Arc::new(IngestExecutor),
        Arc::new(SummarizeExecutor),
        Arc::new(ArchitectExecutor),
        Arc::new(DesignExecutor),
        Arc::new(ImplementExecutor),
        Arc::new(VerifyExecutor),
        Arc::new(DebugExecutor),
        Arc::new(SecurityPatchExecutor),
        Arc::new(DeliverExecutor),
    ]
}

async fn run_architect_finalize(ctx: &StageContext) -> Result<StageOutput> {
    run_architect_finalize_with_answers(ctx, &ctx.architecture_answers).await
}

async fn run_architect_finalize_with_answers(
    ctx: &StageContext,
    answers: &[(String, String)],
) -> Result<StageOutput> {
    let spec = read_named_text(ctx, "project_spec.json").await?;
    let response = ctx
        .openrouter
        .complete(crate::clients::openrouter::AiRequest {
            model: ctx.model_router.model(ModelRole::Plan).to_string(),
            messages: vec![
                crate::clients::openrouter::AiMessage { role: crate::clients::openrouter::AiRole::System, content: "Plan from project_spec.json. Return JSON with architecture, spec, tasks, and planning_meta. Keep tasks small and independent.".into() },
                crate::clients::openrouter::AiMessage { role: crate::clients::openrouter::AiRole::User, content: format!("spec:\n{spec}\nanswers:\n{answers:?}"), },
            ],
            temperature: Some(0.1),
            max_tokens: Some(16_000),
            response_format: Some(crate::clients::openrouter::ResponseFormat { kind: "json_object".into() }),
        })
        .await?;
    let text = response.content.clone();
    let base = format!("projects/{}/architect", ctx.command.project_id.0);
    let spec = ctx
        .artifacts
        .put(
            &format!("{base}/spec.md"),
            Bytes::from(text),
            "text/markdown",
        )
        .await?;

    Ok(StageOutput {
        artifacts: vec![spec],
        metadata: serde_json::json!({
            "phase": "finalize",
            "model": response.model,
            "input_tokens": response.usage.input_tokens,
            "output_tokens": response.usage.output_tokens,
            "cost_usd": response.usage.cost_usd,
        }),
    })
}

fn build_design_prompt(inputs: &[ArtifactRef]) -> String {
    format!(
        "ui_requirements를 반영한 모던 UI 대시보드를 디자인하세요.\n\
         입력: {:?}",
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
    )
}

fn build_implement_prompt(ctx: &StageContext) -> String {
    let inputs = &ctx.input;
    let has_devops = inputs.iter().any(|a| a.name.starts_with("devops_plan"));
    let devops_note = if has_devops {
        "DevOps 계획서에 따라 Containerfile, compose.yml, CI/CD 워크플로우(.github/workflows), \
         nginx/인프라 설정을 구현하세요. 배포 자동화를 포함하세요.\n"
    } else {
        ""
    };
    let lang_note = language_prompt_note(
        ctx.language_mode,
        ctx.programming_language,
        ctx.resolved_language,
    );
    format!(
        "tasks.json 순서대로 구현하세요. design/screens/ 의 UI 참고 자료를 사용하세요. \
         Stitch HTML 또는 Figma PNG/export JSON이 포함될 수 있습니다.\n\
         {lang_note}\
         {devops_note}\
         입력: {:?}",
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
    )
}

fn build_verify_prompt(ctx: &StageContext) -> String {
    format!(
        "구현된 코드베이스에 대해 전체 검증을 수행하세요.\n\
         실행할 검증:\n{}\n\
         모든 테스트·린트·빌드가 통과하면 passed: true.\n\
         strict JSON verify_report 출력: {{ passed, checks: [{{name, passed, output}}], errors: [], summary }}\n\
         PR: {:?}\n\
         이전 산출물: {:?}",
        VERIFY_CHECKS.join("\n"),
        ctx.pr_url,
        ctx.input.iter().map(|a| &a.name).collect::<Vec<_>>()
    )
}

fn build_debug_prompt(
    ctx: &StageContext,
    verify_meta: &serde_json::Value,
    diagnosis: Option<&str>,
) -> String {
    format!(
        "verify_report.json의 실패 항목을 분석하고 자동으로 디버깅·수정하세요.\n\
         1. 실패한 테스트/린트 오류의 근본 원인 파악\n\
         2. 최소 변경으로 수정 (regression 방지)\n\
         3. 수정 후 cargo test / clippy 재실행\n\
         4. strict JSON debug_report 출력: {{ fixes_applied: [], files_changed: [], summary, resolved_errors }}\n\
         Verify 결과: {verify_meta}\n\
         중간 진단(있는 경우)을 최소 패치에 반영하세요: {diagnosis:?}\n\
         PR: {:?}",
        ctx.pr_url
    )
}

fn build_security_prompt(ctx: &StageContext) -> String {
    format!(
        "코드베이스 보안 감사 및 자동 패치를 수행하세요.\n\
         검사 항목:\n{}\n\
         1. 취약한 의존성 업데이트 (cargo audit, npm audit)\n\
         2. OWASP Top 10 코드 취약점 수정 (SQLi, XSS, 인증/인가)\n\
         3. 하드코딩된 시크릿 제거\n\
         4. 패치 후 테스트 재실행\n\
         strict JSON security_report 출력: {{ passed, vulnerabilities_found, patches_applied: [{{id, severity, package, action}}], audit_tools: [], summary }}\n\
         PR: {:?}",
        SECURITY_CHECKS.join("\n"),
        ctx.pr_url
    )
}
