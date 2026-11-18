use crate::clients::figma::FigmaClient;
use crate::clients::omniroute::{CreateAgentOpts, OmniRouteClient};
use crate::clients::stitch::StitchClient;
use crate::domain::{
    ArtifactRef, LanguageMode, PipelineModelConfig, ProgrammingLanguage, StageCommand, StageId,
};
use crate::error::{AutoForgeError, Result};
use crate::services::architecture_qa::parse_clarification_questions;
use crate::services::artifacts::ArtifactStore;
use crate::services::backend_design::{design_instructions, render_response};
use crate::services::ingest::{ingest_devops_file, ingest_pdf};
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
    pub omniroute: Arc<OmniRouteClient>,
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

impl StageContext {
    fn input_named(&self, name: &str) -> Option<&ArtifactRef> {
        self.input.iter().rev().find(|a| a.name == name)
    }

    async fn read_input(&self, name: &str) -> Result<Bytes> {
        let artifact = self
            .input_named(name)
            .ok_or_else(|| AutoForgeError::StageFailed {
                stage: self.command.stage,
                message: format!("missing input {name}"),
            })?;
        self.artifacts.get(&artifact.key).await
    }
}

// Bound each raw-text input while keeping UTF-8 and, where possible, whole lines intact.
fn plan_chunks(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let mut end = text.len().min(64 * 1024);
        if end < text.len() {
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if let Some(newline) = text[..end].rfind('\n').filter(|&pos| pos >= end / 2) {
                end = newline + 1;
            }
        }
        let (chunk, rest) = text.split_at(end);
        text = rest;
        Some(chunk)
    })
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
        let result = tokio::task::spawn_blocking(move || ingest_pdf(&bytes))
            .await
            .map_err(|e| AutoForgeError::Ingest(e.to_string()))??;
        let base = format!("projects/{}/ingest", ctx.command.project_id.0);

        let text_uri = ctx
            .artifacts
            .put(
                &format!("{base}/raw_text.md"),
                Bytes::from(result.raw_text),
                "text/markdown",
            )
            .await?;

        let mut meta = serde_json::json!({
            "page_count": result.page_count,
            "sha256": result.sha256,
            "has_devops_plan": false,
        });

        let mut artifacts = vec![text_uri];

        // DevOps 계획서 ingest (선택)
        let devops_ref = ctx.input.iter().find(|a| a.name.starts_with("devops_plan"));

        if let Some(devops_ref) = devops_ref {
            let devops_bytes = ctx.artifacts.get(&devops_ref.key).await?;
            let filename = devops_ref.name.clone();
            let devops = tokio::task::spawn_blocking(move || {
                ingest_devops_file(&devops_bytes, Some(&filename))
            })
            .await
            .map_err(|e| AutoForgeError::Ingest(e.to_string()))?;
            if let Ok(devops) = devops {
                let devops_uri = ctx
                    .artifacts
                    .put(
                        &format!("{base}/devops_raw_text.md"),
                        Bytes::from(devops.raw_text),
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
        let instructions = build_summarize_prompt(ctx);
        let mut plan = String::new();
        for name in ["raw_text.md", "devops_raw_text.md"] {
            if ctx.input_named(name).is_some() {
                let bytes = ctx.read_input(name).await?;
                plan.push_str(&format!("\n{name} 본문\n"));
                plan.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        let profile = ctx.model_config.profile_for(StageId::Summarize);

        if plan.is_empty() {
            return Err(AutoForgeError::Ingest("missing extracted plan text".into()));
        }
        let mut text = String::new();
        let mut run_id = String::new();
        for chunk in plan_chunks(&plan) {
            let prompt = format!("{instructions}\n기존 요약의 요구사항·부하 수치·제약을 유지하고 이번 본문을 통합한 전체 요약을 반환하세요.\n기존 요약\n{text}\n추가 본문\n{chunk}");
            let resp = ctx
                .omniroute
                .create_agent(&prompt, &profile, CreateAgentOpts::default())
                .await?;
            let run = ctx
                .omniroute
                .wait_for_run(
                    &resp.agent.id,
                    &resp.run.id,
                    std::time::Duration::from_secs(5),
                )
                .await?;
            text = run
                .result_text()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| AutoForgeError::StageFailed {
                    stage: StageId::Summarize,
                    message: "empty agent response".into(),
                })?;
            run_id = resp.run.id;
        }

        let resolved =
            resolve_effective_language(ctx.language_mode, ctx.programming_language, &text);

        let base = format!("projects/{}/summarize", ctx.command.project_id.0);
        let artifact = ctx
            .artifacts
            .put(
                &format!("{base}/summary.json"),
                Bytes::from(text),
                "application/json",
            )
            .await?;

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "omniroute_run_id": run_id,
                "programming_language": resolved.as_str(),
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
        let bytes = ctx.read_input("summary.json").await?;
        let summary = String::from_utf8_lossy(&bytes);
        if ctx.architecture_finalize {
            return run_architect_finalize(ctx, &summary).await;
        }

        let prompt = format!(
            "{}\nsummary.json\n{summary}",
            build_architect_draft_prompt(ctx)
        );
        let profile = ctx.model_config.profile_for(StageId::Architect);

        let resp = ctx
            .omniroute
            .create_agent(&prompt, &profile, CreateAgentOpts::default())
            .await?;

        let run = ctx
            .omniroute
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(10),
            )
            .await?;

        let text = run.result_text().unwrap_or_default();
        let questions = parse_clarification_questions(&text)?;

        if questions.is_empty() {
            return run_architect_finalize(ctx, &summary).await;
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
                "omniroute_run_id": resp.run.id,
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

        let mut prompt = build_implement_prompt(ctx);
        if ctx.input_named("backend.hcl").is_some() {
            prompt.push_str("\nbackend.hcl에 따라 컴포넌트와 채택한 최적화를 구현하세요. 기존 인덱스를 확인하고 필요한 마이그레이션과 대표 조회의 실행 계획을 검증하세요.");
        }
        let profile = ctx.model_config.profile_for(StageId::Implement);

        let opts = CreateAgentOpts {
            repo_url: Some(repo_url),
            starting_ref: Some("main"),
            auto_create_pr: Some(true),
            agent_id: None,
            pr_url: None,
            input_artifacts: &ctx.input,
        };

        let resp = ctx.omniroute.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .omniroute
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
                "omniroute_run_id": resp.run.id,
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
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref(), &ctx.input);

        let resp = ctx.omniroute.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .omniroute
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
                "omniroute_run_id": resp.run.id,
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

        let prompt = build_debug_prompt(ctx, &verify_meta);
        let profile = ctx.model_config.profile_for(StageId::Debug);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref(), &ctx.input);

        let resp = ctx.omniroute.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .omniroute
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
                "omniroute_run_id": resp.run.id,
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
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref(), &ctx.input);

        let resp = ctx.omniroute.create_agent(&prompt, &profile, opts).await?;
        let run = ctx
            .omniroute
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
                "omniroute_run_id": resp.run.id,
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

fn agent_opts<'a>(
    repo_url: &'a str,
    pr_url: Option<&'a str>,
    input_artifacts: &'a [ArtifactRef],
) -> CreateAgentOpts<'a> {
    CreateAgentOpts {
        repo_url: Some(repo_url),
        starting_ref: Some("main"),
        auto_create_pr: Some(false),
        agent_id: None,
        pr_url,
        input_artifacts,
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

fn build_summarize_prompt(ctx: &StageContext) -> String {
    let inputs = &ctx.input;
    let has_devops = inputs.iter().any(|a| a.name.starts_with("devops_plan"));
    let devops_note = if has_devops {
        "DevOps 계획서(devops_plan*)가 포함되어 있습니다. devops_requirements, infrastructure, ci_cd 필드를 반드시 채우세요.\n"
    } else {
        ""
    };
    let lang_note = language_prompt_note(
        ctx.language_mode,
        ctx.programming_language,
        ctx.resolved_language,
    );
    format!(
        "다음 외주 계획서를 분석하여 strict JSON으로 요약하세요.\n\
         필드: title, goals[], scope, constraints[], tech_hints[], ui_requirements[], \
         devops_requirements[], infrastructure[], ci_cd[], timeline, budget_hint, workload(사용자 수, 피크 부하, 데이터량, 지연 목표의 수치·단위·출처 또는 미상), \
         programming_language, language_rationale\n\
         programming_language: 프로젝트에 가장 적합한 주 구현 언어 (rust|typescript|python|go|java|kotlin|swift|csharp|ruby|php)\n\
         {lang_note}\
         {devops_note}\
         입력 아티팩트: {:?}",
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
    )
}

fn build_architect_draft_prompt(ctx: &StageContext) -> String {
    let inputs = &ctx.input;
    let has_devops = inputs.iter().any(|a| a.name.starts_with("devops_plan"));
    let devops_note = if has_devops {
        "DevOps 계획서를 고려하여 인프라/배포 관련 질문도 포함하세요.\n"
    } else {
        ""
    };
    let lang_note = language_prompt_note(
        ctx.language_mode,
        ctx.programming_language,
        ctx.resolved_language,
    );
    format!(
        "summary.json을 분석하여 아키텍처 설계 전에 사용자에게 확인이 필요한 질문 목록을 작성하세요.\n\
         {lang_note}\
         {devops_note}\
         strict JSON만 출력하세요:\n\
         {{\"questions\":[{{\"id\":\"q1\",\"text\":\"질문\",\"options\":[\"선택지1\",\"선택지2\"],\"required\":true,\"category\":\"database|auth|scale|stack|other\"}}]}}\n\
         - 모호한 요구사항, 기술 스택 선택, 데이터 모델, 피크 부하, 지연 목표, 배포 예산 등을 3~7개 질문으로 정리\n\
         - options가 있으면 사용자가 선택할 수 있게 제공\n\
         - 명확한 계획서라면 questions를 빈 배열로 반환 가능\n\
         입력: {:?}",
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
    )
}

async fn run_architect_finalize(ctx: &StageContext, summary: &str) -> Result<StageOutput> {
    let prompt = build_architect_finalize_prompt(ctx, &ctx.architecture_answers, summary);
    let profile = ctx.model_config.profile_for(StageId::Architect);

    let resp = ctx
        .omniroute
        .create_agent(&prompt, &profile, CreateAgentOpts::default())
        .await?;

    let run = ctx
        .omniroute
        .wait_for_run(
            &resp.agent.id,
            &resp.run.id,
            std::time::Duration::from_secs(10),
        )
        .await?;

    let text = run.result_text().unwrap_or_default();
    let (spec_md, hcl) = render_response(&text)?;
    let base = format!("projects/{}/architect", ctx.command.project_id.0);
    let spec = ctx
        .artifacts
        .put(
            &format!("{base}/spec.md"),
            Bytes::from(spec_md),
            "text/markdown",
        )
        .await?;
    let design = ctx
        .artifacts
        .put(
            &format!("{base}/backend.hcl"),
            Bytes::from(hcl),
            "text/plain; charset=utf-8",
        )
        .await?;

    Ok(StageOutput {
        artifacts: vec![spec, design],
        metadata: serde_json::json!({
            "phase": "finalize",
            "backend_design": "backend.hcl",
            "omniroute_run_id": resp.run.id,
        }),
    })
}

fn build_architect_finalize_prompt(
    ctx: &StageContext,
    answers: &[(String, String)],
    summary: &str,
) -> String {
    let inputs = &ctx.input;
    let has_devops = inputs.iter().any(|a| a.name.starts_with("devops_plan"));
    let devops_note = if has_devops {
        "DevOps 계획서를 반영하여 infrastructure.md, cicd_pipeline.md, deploy_manifest.yaml 초안도 포함하세요.\n"
    } else {
        ""
    };
    let lang_note = language_prompt_note(
        ctx.language_mode,
        ctx.programming_language,
        ctx.resolved_language,
    );
    let answers_note = if answers.is_empty() {
        "추가 질의응답 없이 summary.json만으로 설계하세요.\n".to_string()
    } else {
        let lines: Vec<String> = answers
            .iter()
            .map(|(id, ans)| format!("- {id}: {ans}"))
            .collect();
        format!(
            "사용자가 아래 질문에 답변했습니다. 반드시 반영하세요:\n{}\n",
            lines.join("\n")
        )
    };
    format!(
        "summary.json과 사용자 답변을 기반으로 시스템 아키텍처(architecture.md)와 상세 기획(spec.md), \
         구현 태스크 DAG(tasks.json)를 작성하세요.\n\
         {lang_note}\
         {devops_note}\
         {answers_note}\n{}\n입력 summary.json\n{summary}",
        design_instructions()
    )
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
         {}\
         입력: {:?}",
        ctx.model_config
            .coding_skills
            .as_ref()
            .map(|s| s.prompt_note())
            .unwrap_or_default(),
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
    )
}

fn build_verify_prompt(ctx: &StageContext) -> String {
    format!(
        "{}구현된 코드베이스에 대해 전체 검증을 수행하세요.\n\
         실행할 검증:\n{}\n\
         모든 테스트·린트·빌드가 통과하면 passed: true.\n\
         strict JSON verify_report 출력: {{ passed, checks: [{{name, passed, output}}], errors: [], summary }}\n\
         PR: {:?}\n\
         이전 산출물: {:?}",
        ctx.model_config.coding_skills.as_ref().map(|s| s.prompt_note()).unwrap_or_default(),
        VERIFY_CHECKS.join("\n"),
        ctx.pr_url,
        ctx.input.iter().map(|a| &a.name).collect::<Vec<_>>()
    )
}

fn build_debug_prompt(ctx: &StageContext, verify_meta: &serde_json::Value) -> String {
    format!(
        "{}verify_report.json의 실패 항목을 분석하고 자동으로 디버깅·수정하세요.\n\
         1. 실패한 테스트/린트 오류의 근본 원인 파악\n\
         2. 최소 변경으로 수정 (regression 방지)\n\
         3. 수정 후 cargo test / clippy 재실행\n\
         4. strict JSON debug_report 출력: {{ fixes_applied: [], files_changed: [], summary, resolved_errors }}\n\
         Verify 결과: {verify_meta}\n\
         PR: {:?}",
        ctx.model_config.coding_skills.as_ref().map(|s| s.prompt_note()).unwrap_or_default(),
        ctx.pr_url
    )
}

fn build_security_prompt(ctx: &StageContext) -> String {
    format!(
        "{}코드베이스 보안 감사 및 자동 패치를 수행하세요.\n\
         검사 항목:\n{}\n\
         1. 취약한 의존성 업데이트 (cargo audit, npm audit)\n\
         2. OWASP Top 10 코드 취약점 수정 (SQLi, XSS, 인증/인가)\n\
         3. 하드코딩된 시크릿 제거\n\
         4. 패치 후 테스트 재실행\n\
         strict JSON security_report 출력: {{ passed, vulnerabilities_found, patches_applied: [{{id, severity, package, action}}], audit_tools: [], summary }}\n\
         PR: {:?}",
        ctx.model_config.coding_skills.as_ref().map(|s| s.prompt_note()).unwrap_or_default(),
        SECURITY_CHECKS.join("\n"),
        ctx.pr_url
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_plans_keep_middle_sections_and_multibyte_text() {
        let text = format!(
            "{}\n피크 부하 120 RPS\n{}",
            "가".repeat(33000),
            "나".repeat(33000)
        );
        let chunks: Vec<_> = plan_chunks(&text).collect();
        assert!(chunks.len() > 2);
        assert_eq!(chunks.concat(), text);
        assert!(chunks
            .iter()
            .all(|chunk| !chunk.is_empty() && chunk.len() <= 64 * 1024));
    }
}
