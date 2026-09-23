use super::{agent_opts, StageContext, StageExecutor, StageOutput};
use crate::domain::StageId;
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{complete_json, parse_json, AiPurpose};
use crate::services::quality::{
    DebugReport, SecurityReport, VerifyReport, MAX_DEBUG_CYCLES, SECURITY_CHECKS, VERIFY_CHECKS,
};
use async_trait::async_trait;
use bytes::Bytes;

mod debug_context;
mod diagnosis;
use diagnosis::Diagnosis;

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
        let profile = ctx.model_config.profile_for(StageId::Verify);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref());
        let response = ctx
            .cursor
            .create_agent(&build_verify_prompt(ctx), &profile, opts)
            .await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &response.agent.id,
                &response.run.id,
                std::time::Duration::from_secs(15),
            )
            .await?;
        let report = VerifyReport::parse_from_agent_text(&run.result_text().unwrap_or_default());
        let key = format!(
            "projects/{}/verify/verify_report.json",
            ctx.command.project_id.0
        );
        let artifact = ctx
            .artifacts
            .put(
                &key,
                Bytes::from(serde_json::to_string(&report).unwrap_or_default()),
                "application/json",
            )
            .await?;
        Ok(StageOutput {
            artifacts: vec![artifact],
            metadata: serde_json::json!({
                "passed": report.passed,
                "errors": report.errors.len(),
                "cursor_agent_id": response.agent.id,
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
        let max_attempts = ctx
            .deepseek_debug_retries
            .saturating_add(ctx.mid_debug_retries)
            .saturating_add(ctx.opus_max_calls)
            .min(MAX_DEBUG_CYCLES);
        if ctx.command.attempt >= max_attempts {
            return Err(AutoForgeError::StageFailed {
                stage: StageId::Debug,
                message: format!(
                    "debug attempt {} exceeds the configured maximum of {max_attempts}",
                    ctx.command.attempt
                ),
            });
        }
        let repo_url = ctx
            .repo_url
            .as_deref()
            .ok_or_else(|| AutoForgeError::BadRequest("repo_url required for debug".into()))?;
        let evidence =
            debug_context::load(ctx.artifacts.as_ref(), &ctx.command.project_id, &ctx.input)
                .await?;
        let evidence_json = evidence.json()?;
        let role = ctx.model_router.debug_role(
            ctx.command.attempt,
            ctx.deepseek_debug_retries,
            ctx.mid_debug_retries,
        );
        let diagnosis = if ctx.command.attempt >= ctx.deepseek_debug_retries {
            let response = complete_json(
                ctx.omniroute.as_ref(),
                ctx.model_router.model(role),
                "Diagnose the verification failure only. Return JSON with exactly root_cause, affected_files, recommended_fix, risk, and additional_tests. Do not propose or output code rewrites.",
                evidence_json.clone(),
                AiPurpose::Diagnosis,
                &ctx.token_policy,
            )
            .await?;
            let diagnosis: Diagnosis = parse_json(&response.content)?;
            Some((diagnosis.bounded(), response.model, response.usage))
        } else {
            None
        };
        let profile = ctx.model_config.profile_for(StageId::Debug);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref());
        let response = ctx
            .cursor
            .create_agent(
                &build_debug_prompt(
                    ctx,
                    &evidence_json,
                    diagnosis.as_ref().map(|value| &value.0),
                ),
                &profile,
                opts,
            )
            .await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &response.agent.id,
                &response.run.id,
                std::time::Duration::from_secs(20),
            )
            .await?;
        let text = run.result_text().unwrap_or_default();
        let report = DebugReport {
            fixes_applied: vec![],
            files_changed: vec![],
            summary: text.chars().take(300).collect(),
            resolved_errors: 0,
        };
        let key = format!(
            "projects/{}/debug/debug_report.json",
            ctx.command.project_id.0
        );
        let artifact = ctx
            .artifacts
            .put(
                &key,
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
                "cursor_agent_id": response.agent.id,
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
        let profile = ctx.model_config.profile_for(StageId::SecurityPatch);
        let opts = agent_opts(repo_url, ctx.pr_url.as_deref());
        let response = ctx
            .cursor
            .create_agent(&build_security_prompt(ctx), &profile, opts)
            .await?;
        let run = ctx
            .cursor
            .wait_for_run(
                &response.agent.id,
                &response.run.id,
                std::time::Duration::from_secs(20),
            )
            .await?;
        let report = SecurityReport::parse_from_agent_text(&run.result_text().unwrap_or_default());
        let key = format!(
            "projects/{}/security/security_report.json",
            ctx.command.project_id.0
        );
        let artifact = ctx
            .artifacts
            .put(
                &key,
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
                "cursor_agent_id": response.agent.id,
            }),
        })
    }
}

fn build_verify_prompt(ctx: &StageContext) -> String {
    format!(
        "구현된 코드베이스에 대해 전체 검증을 수행하세요.\n실행할 검증:\n{}\n모든 테스트·린트·빌드가 통과하면 passed: true.\nstrict JSON verify_report 출력: {{ passed, checks: [{{name, passed, output}}], errors: [], summary }}\nPR: {:?}\n이전 산출물: {:?}",
        VERIFY_CHECKS.join("\n"),
        ctx.pr_url,
        ctx.input.iter().map(|artifact| &artifact.name).collect::<Vec<_>>()
    )
}

fn build_debug_prompt(
    ctx: &StageContext,
    evidence_json: &str,
    diagnosis: Option<&Diagnosis>,
) -> String {
    format!(
        "verify_report.json의 실패 항목을 분석하고 자동으로 디버깅·수정하세요.\n1. 실패한 테스트/린트 오류의 근본 원인 파악\n2. 최소 변경으로 수정 (regression 방지)\n3. 수정 후 cargo test / clippy 재실행\n4. strict JSON debug_report 출력: {{ fixes_applied: [], files_changed: [], summary, resolved_errors }}\n압축된 verify 증거: {evidence_json}\n중간 진단(있는 경우)을 최소 패치에 반영하세요: {diagnosis:?}\nPR: {:?}",
        ctx.pr_url
    )
}

fn build_security_prompt(ctx: &StageContext) -> String {
    format!(
        "코드베이스 보안 감사 및 자동 패치를 수행하세요.\n검사 항목:\n{}\n1. 취약한 의존성 업데이트 (cargo audit, npm audit)\n2. OWASP Top 10 코드 취약점 수정 (SQLi, XSS, 인증/인가)\n3. 하드코딩된 시크릿 제거\n4. 패치 후 테스트 재실행\nstrict JSON security_report 출력: {{ passed, vulnerabilities_found, patches_applied: [{{id, severity, package, action}}], audit_tools: [], summary }}\nPR: {:?}",
        SECURITY_CHECKS.join("\n"),
        ctx.pr_url
    )
}
