use super::extraction_cache::usage_metadata;
use super::{gateway_model, read_named_text, StageContext, StageExecutor, StageOutput};
use crate::clients::model_router::ModelRole;
use crate::clients::omniroute::AiResponse;
use crate::domain::{ArtifactRef, StageId};
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{complete_json, parse_json, AiPurpose, ProjectSpec, QuestionList};
use crate::services::planning::{escalation_needed, parse_and_validate, PlanningBundle};
use async_trait::async_trait;
use bytes::Bytes;
use serde::Serialize;

const VALIDATION_RETRIES: u8 = 2;

pub struct ArchitectExecutor;

#[async_trait]
impl StageExecutor for ArchitectExecutor {
    fn stage(&self) -> StageId {
        StageId::Architect
    }

    async fn execute(&self, ctx: &StageContext) -> Result<StageOutput> {
        if ctx.architecture_finalize {
            return finalize(ctx).await;
        }
        request_clarifications(ctx).await
    }
}

async fn request_clarifications(ctx: &StageContext) -> Result<StageOutput> {
    let project_spec = read_project_spec(ctx).await?;
    let response = complete_json(
        ctx.omniroute.as_ref(),
        gateway_model(
            ctx.model_config.architect.as_deref(),
            ctx.model_router.model(ModelRole::Plan),
        ),
        "Create only necessary clarification questions from project_spec. Do not invent requirements. Return {\"questions\":[{\"id\":string,\"question\":string,\"options\":[string],\"required\":boolean,\"category\":string|null}]}",
        planning_input(&project_spec, &[], None)?,
        AiPurpose::Clarify,
        &ctx.token_policy,
    )
    .await?;
    let questions = parse_json::<QuestionList>(&response.content)?.questions;

    if questions.is_empty() {
        return finalize_with_answers(ctx, &[], vec![response]).await;
    }

    let artifact = store_json(
        ctx,
        "clarifications.json",
        &serde_json::json!({ "questions": questions }),
    )
    .await?;
    let question_views: Vec<_> = questions
        .iter()
        .map(|question| {
            serde_json::json!({
                "id": question.id,
                "question": question.question,
                "options": question.options,
                "required": question.required,
                "category": question.category,
            })
        })
        .collect();

    let mut metadata = usage_metadata(std::slice::from_ref(&response));
    metadata["phase"] = serde_json::json!("draft");
    metadata["model"] = serde_json::json!(response.model);
    metadata["questions"] = serde_json::json!(question_views);
    metadata["question_count"] = serde_json::json!(questions.len());
    Ok(StageOutput {
        artifacts: vec![artifact],
        metadata,
    })
}

async fn finalize(ctx: &StageContext) -> Result<StageOutput> {
    finalize_with_answers(ctx, &ctx.architecture_answers, vec![]).await
}

async fn finalize_with_answers(
    ctx: &StageContext,
    answers: &[(String, String)],
    mut responses: Vec<AiResponse>,
) -> Result<StageOutput> {
    let project_spec = read_project_spec(ctx).await?;
    let generated = generate_valid_plan(ctx, &project_spec, answers, &mut responses).await?;
    let artifacts = persist_plan(ctx, &generated.bundle).await?;

    let mut metadata = usage_metadata(&responses);
    metadata["phase"] = serde_json::json!("finalize");
    metadata["model"] = serde_json::json!(responses.last().map(|response| &response.model));
    metadata["validation_attempts"] = serde_json::json!(generated.validation_attempts);
    metadata["escalated"] = serde_json::json!(generated.escalated);
    Ok(StageOutput {
        artifacts,
        metadata,
    })
}

async fn generate_valid_plan(
    ctx: &StageContext,
    project_spec: &ProjectSpec,
    answers: &[(String, String)],
    responses: &mut Vec<AiResponse>,
) -> Result<GeneratedPlan> {
    let mut validation_error = None;
    let mut validated = None;

    for attempt in 0..VALIDATION_RETRIES {
        let response = complete_json(
            ctx.omniroute.as_ref(),
            gateway_model(
                ctx.model_config.architect.as_deref(),
                ctx.model_router.model(ModelRole::Plan),
            ),
            plan_system_prompt(),
            planning_input(project_spec, answers, validation_error.as_deref())?,
            AiPurpose::Plan,
            &ctx.token_policy,
        )
        .await?;
        let parsed = parse_and_validate(&response.content);
        responses.push(response);
        match parsed {
            Ok(bundle) => {
                validated = Some(GeneratedPlan {
                    bundle,
                    validation_attempts: attempt + 1,
                    escalated: false,
                });
                break;
            }
            Err(error) => validation_error = Some(error),
        }
    }

    let validation_exhausted = validated.is_none();
    if !escalation_needed(
        project_spec,
        validated.as_ref().map(|result| &result.bundle),
        validation_exhausted,
    ) {
        return validated.ok_or_else(|| AutoForgeError::OmniRoute("plan validation failed".into()));
    }
    if ctx.astra_max_calls == 0 {
        return Err(AutoForgeError::OmniRoute(
            "plan escalation is required but AI_ASTRA_MAX_CALLS is zero".into(),
        ));
    }

    let escalation_input = serde_json::json!({
        "project_spec": project_spec,
        "answers": answers,
        "previous_plan": validated.as_ref().map(|plan| &plan.bundle),
        "conflicts": project_spec.contradictions,
        "validation_feedback": validation_error,
    })
    .to_string();
    let response = complete_json(
        ctx.omniroute.as_ref(),
        ctx.model_router.model(ModelRole::PlanEscalation),
        plan_system_prompt(),
        escalation_input,
        AiPurpose::Plan,
        &ctx.token_policy,
    )
    .await?;
    let bundle = parse_and_validate(&response.content).map_err(invalid_plan)?;
    responses.push(response);
    Ok(GeneratedPlan {
        bundle,
        validation_attempts: VALIDATION_RETRIES,
        escalated: true,
    })
}

fn invalid_plan(error: String) -> AutoForgeError {
    AutoForgeError::OmniRoute(format!("invalid architect plan: {error}"))
}

async fn read_project_spec(ctx: &StageContext) -> Result<ProjectSpec> {
    let raw = read_named_text(ctx, "project_spec.json").await?;
    parse_json(&raw)
}

fn planning_input(
    project_spec: &ProjectSpec,
    answers: &[(String, String)],
    validation_feedback: Option<&str>,
) -> Result<String> {
    let answers = answers
        .iter()
        .map(|(id, answer)| serde_json::json!({ "id": id, "answer": answer }))
        .collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({
        "project_spec": project_spec,
        "answers": answers,
        "validation_feedback": validation_feedback,
    }))
    .map_err(|error| AutoForgeError::OmniRoute(error.to_string()))
}

async fn persist_plan(ctx: &StageContext, plan: &PlanningBundle) -> Result<Vec<ArtifactRef>> {
    let base = format!("projects/{}/architect", ctx.command.project_id.0);
    let architecture = ctx
        .artifacts
        .put(
            &format!("{base}/architecture.md"),
            Bytes::from(plan.architecture.content.clone()),
            "text/markdown",
        )
        .await?;
    let spec = ctx
        .artifacts
        .put(
            &format!("{base}/spec.md"),
            Bytes::from(plan.spec.content.clone()),
            "text/markdown",
        )
        .await?;
    let tasks = store_json(ctx, "tasks.json", &serde_json::json!({"tasks": plan.tasks})).await?;
    let planning_meta = store_json(ctx, "planning_meta.json", &plan.planning_meta).await?;
    Ok(vec![architecture, spec, tasks, planning_meta])
}

async fn store_json<T: Serialize>(
    ctx: &StageContext,
    name: &str,
    value: &T,
) -> Result<ArtifactRef> {
    let data =
        serde_json::to_vec(value).map_err(|error| AutoForgeError::OmniRoute(error.to_string()))?;
    ctx.artifacts
        .put(
            &format!("projects/{}/architect/{name}", ctx.command.project_id.0),
            Bytes::from(data),
            "application/json",
        )
        .await
}

fn plan_system_prompt() -> &'static str {
    "Return only JSON: {architecture:{title:string,content:string},spec:{title:string,content:string},tasks:[{id:string,title:string,type:string,dependencies:[string],acceptance_criteria:[string],risk:string,estimated_context:[string],status:\"pending\"}],planning_meta:{complexity:string,confidence:number,requires_escalation:boolean,reason:string|null}}. Architecture and spec content must be non-blank Markdown. At least one task is required; IDs and titles must be non-blank, acceptance criteria non-empty, and dependencies must reference task IDs in an acyclic DAG. estimated_context contains relevant file hints, not source contents. confidence must be 0.0 through 1.0. Keep tasks small and independently actionable. Do not add requirements absent from the source spec."
}

struct GeneratedPlan {
    bundle: PlanningBundle,
    validation_attempts: u8,
    escalated: bool,
}
