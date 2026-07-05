use crate::clients::cursor::{CreateAgentOpts, CursorClient};
use crate::clients::stitch::StitchClient;
use crate::domain::{
    ArtifactRef, LanguageMode, PipelineModelConfig, ProgrammingLanguage, StageCommand, StageId,
};
use crate::error::{AutoForgeError, Result};
use crate::services::architecture_qa::parse_clarification_questions;
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
    pub stitch: Arc<StitchClient>,
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
        let prompt = build_summarize_prompt(ctx);
        let profile = ctx.model_config.profile_for(StageId::Summarize);

        let resp = ctx
            .cursor
            .create_agent(&prompt, &profile, CreateAgentOpts::default())
            .await?;

        let run = ctx
            .cursor
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(5),
            )
            .await?;

        let text = run
            .result
            .and_then(|r| r.text)
            .ok_or_else(|| AutoForgeError::StageFailed {
                stage: StageId::Summarize,
                message: "empty agent response".into(),
            })?;

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
                "cursor_agent_id": resp.agent.id,
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
        if ctx.architecture_finalize {
            return run_architect_finalize(ctx).await;
        }

        let prompt = build_architect_draft_prompt(ctx);
        let profile = ctx.model_config.profile_for(StageId::Architect);

        let resp = ctx
            .cursor
            .create_agent(&prompt, &profile, CreateAgentOpts::default())
            .await?;

        let run = ctx
            .cursor
            .wait_for_run(
                &resp.agent.id,
                &resp.run.id,
                std::time::Duration::from_secs(10),
            )
            .await?;

        let text = run.result.and_then(|r| r.text).unwrap_or_default();
        let questions = parse_clarification_questions(&text).unwrap_or_default();

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
                "cursor_agent_id": resp.agent.id,
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
        let prompt = build_design_prompt(&ctx.input);
        let device_type = ctx.model_config.design_device_type();
        let screen = ctx.stitch.generate_screen(&prompt, device_type).await?;
        let html = ctx.stitch.get_screen_html(&screen.id).await?;

        let artifact = ArtifactRef {
            name: format!("screens/{}.html", screen.id),
            key: html.download_url.clone(),
            uri: html.download_url,
            content_type: "text/html".into(),
            sha256: None,
        };

        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({ "screen_id": screen.id }),
        })
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

        let text = run.result.and_then(|r| r.text).unwrap_or_default();
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

        let prompt = build_debug_prompt(ctx, &verify_meta);
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

        let text = run.result.and_then(|r| r.text).unwrap_or_default();
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

        let text = run.result.and_then(|r| r.text).unwrap_or_default();
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
         devops_requirements[], infrastructure[], ci_cd[], timeline, budget_hint, \
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
         - 모호한 요구사항, 기술 스택 선택, 데이터 모델, 인증 방식, 배포 환경 등을 3~7개 질문으로 정리\n\
         - options가 있으면 사용자가 선택할 수 있게 제공\n\
         - 명확한 계획서라면 questions를 빈 배열로 반환 가능\n\
         입력: {:?}",
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
    )
}

async fn run_architect_finalize(ctx: &StageContext) -> Result<StageOutput> {
    run_architect_finalize_with_answers(ctx, &ctx.architecture_answers).await
}

async fn run_architect_finalize_with_answers(
    ctx: &StageContext,
    answers: &[(String, String)],
) -> Result<StageOutput> {
    let prompt = build_architect_finalize_prompt(ctx, answers);
    let profile = ctx.model_config.profile_for(StageId::Architect);

    let resp = ctx
        .cursor
        .create_agent(&prompt, &profile, CreateAgentOpts::default())
        .await?;

    let run = ctx
        .cursor
        .wait_for_run(
            &resp.agent.id,
            &resp.run.id,
            std::time::Duration::from_secs(10),
        )
        .await?;

    let text = run.result.and_then(|r| r.text).unwrap_or_default();
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
            "cursor_agent_id": resp.agent.id,
        }),
    })
}

fn build_architect_finalize_prompt(ctx: &StageContext, answers: &[(String, String)]) -> String {
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
         {answers_note}\
         입력: {:?}",
        inputs.iter().map(|a| &a.uri).collect::<Vec<_>>()
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
        "tasks.json 순서대로 구현하세요. design/screens/ 의 Stitch HTML을 UI 참고로 사용하세요.\n\
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

fn build_debug_prompt(ctx: &StageContext, verify_meta: &serde_json::Value) -> String {
    format!(
        "verify_report.json의 실패 항목을 분석하고 자동으로 디버깅·수정하세요.\n\
         1. 실패한 테스트/린트 오류의 근본 원인 파악\n\
         2. 최소 변경으로 수정 (regression 방지)\n\
         3. 수정 후 cargo test / clippy 재실행\n\
         4. strict JSON debug_report 출력: {{ fixes_applied: [], files_changed: [], summary, resolved_errors }}\n\
         Verify 결과: {verify_meta}\n\
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
