use crate::app::App;
use crate::domain::{
    ArchitectureClarification, ArtifactRef, LanguageMode, PipelineState, ProgrammingLanguage,
    Project, StageCompleted, StageId, StageState,
};
use crate::error::{AutoForgeError, Result};
use crate::services::architecture_qa::all_required_answered;
use crate::services::daily_log::DailyEvent;
use crate::services::daily_log_notify::record_daily_event;
use crate::services::github::try_auto_merge_pr;
use crate::services::worker::{executors, StageContext, StageOutput};
use bytes::Bytes;
use tracing::{info, warn};
use uuid::Uuid;

/// 단일 스테이지 실행
pub async fn execute_stage(app: &App, project: &Project, stage: StageId) -> Result<StageOutput> {
    let project_id = project.id.0;
    let pdf_key = format!("projects/{project_id}/plan.pdf");

    let mut accumulated = project.accumulated_artifacts.clone();
    if accumulated.is_empty() {
        accumulated.push(ArtifactRef {
            name: "plan.pdf".into(),
            key: pdf_key.clone(),
            uri: app.artifacts.uri_for(&pdf_key),
            content_type: "application/pdf".into(),
            sha256: None,
        });
    }

    let executor_map: std::collections::HashMap<_, _> =
        executors().into_iter().map(|e| (e.stage(), e)).collect();

    let executor = executor_map
        .get(&stage)
        .ok_or_else(|| AutoForgeError::Internal(format!("no executor for {stage:?}")))?;

    let pr_url = project
        .stage_outputs
        .get(&StageId::Implement)
        .and_then(|m| m.get("pr_url"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let architecture_finalize = project.scheduler.architecture.draft_done
        && !project.scheduler.architecture.awaiting_answers
        && !project.scheduler.architecture.finalized;

    let architecture_answers: Vec<(String, String)> = project
        .architecture_clarifications
        .iter()
        .filter_map(|q| q.answer.as_ref().map(|a| (q.id.clone(), a.clone())))
        .collect();

    let ctx = StageContext {
        command: crate::domain::StageCommand {
            project_id: project.id.clone(),
            stage,
            attempt: 0,
        },
        artifacts: app.artifacts.clone(),
        cursor: app.cursor.clone(),
        stitch: app.stitch.clone(),
        input: accumulated,
        repo_url: project
            .repo_url
            .clone()
            .or(app.config.default_repo_url.clone()),
        stage_outputs: project.stage_outputs.clone(),
        pr_url,
        language_mode: project.language_mode,
        programming_language: project.programming_language,
        resolved_language: project.resolved_language,
        architecture_finalize,
        architecture_answers,
    };

    executor.execute(&ctx).await
}

/// 스테이지 결과를 프로젝트에 반영 (GitHub 머지는 SecurityPatch 이후 비동기 처리)
pub async fn apply_stage_output_async(
    app: &App,
    project: &mut Project,
    stage: StageId,
    output: StageOutput,
) -> Result<PipelineOutcome> {
    let outcome = apply_stage_output(project, stage, output)?;

    if stage == StageId::SecurityPatch {
        try_auto_merge_pr(app, project).await?;
    }

    Ok(outcome)
}

/// 스테이지 결과를 프로젝트에 반영
pub fn apply_stage_output(
    project: &mut Project,
    stage: StageId,
    output: StageOutput,
) -> Result<PipelineOutcome> {
    project
        .accumulated_artifacts
        .extend(output.artifacts.clone());
    project.stage_outputs.insert(stage, output.metadata.clone());

    match stage {
        StageId::Summarize => {
            if project.language_mode == LanguageMode::Manual {
                if let Some(lang) = project.programming_language {
                    project.resolved_language = Some(lang);
                }
            } else if let Some(lang_str) = output
                .metadata
                .get("programming_language")
                .and_then(|v| v.as_str())
            {
                if let Some(lang) = ProgrammingLanguage::from_str_loose(lang_str) {
                    project.resolved_language = Some(lang);
                }
            }
            project.stages.insert(stage, StageState::Completed);
            project.scheduler.mark_completed(&StageCompleted {
                project_id: project.id.clone(),
                stage,
                output_artifacts: output.artifacts,
            });
        }
        StageId::Architect => {
            let phase = output
                .metadata
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("finalize");

            if phase == "draft" {
                project.architecture_clarifications = parse_architect_questions(&output.metadata);
                project.scheduler.record_architect_draft();
                project.stages.insert(stage, StageState::Running);
                project.state = PipelineState::AwaitingInput;
                return Ok(PipelineOutcome::AwaitingInput);
            }

            project.scheduler.record_architect_finalized();
            project.stages.insert(stage, StageState::Completed);
            project.scheduler.mark_completed(&StageCompleted {
                project_id: project.id.clone(),
                stage,
                output_artifacts: output.artifacts,
            });
        }
        StageId::Verify => {
            let passed = output
                .metadata
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            project.scheduler.record_verify_result(passed);
            if passed {
                project.stages.insert(stage, StageState::Completed);
            } else if project.scheduler.quality.verify_exhausted() {
                project.stages.insert(stage, StageState::Failed);
                project.scheduler.mark_failed(stage);
                project.state = PipelineState::Failed;
                return Ok(PipelineOutcome::Failed(
                    "verification failed after max debug cycles".into(),
                ));
            } else {
                project.stages.insert(stage, StageState::Failed);
                project.stages.insert(StageId::Debug, StageState::Queued);
            }
        }
        StageId::Debug => {
            project.scheduler.record_debug_done();
            project.stages.insert(stage, StageState::Completed);
            project.stages.insert(StageId::Verify, StageState::Queued);
        }
        StageId::SecurityPatch => {
            project.scheduler.record_security_done();
            project.stages.insert(stage, StageState::Completed);
        }
        _ => {
            project.stages.insert(stage, StageState::Completed);
            project.scheduler.mark_completed(&StageCompleted {
                project_id: project.id.clone(),
                stage,
                output_artifacts: output.artifacts,
            });
        }
    }

    if project.scheduler.is_pipeline_complete() {
        project.state = PipelineState::Completed;
        return Ok(PipelineOutcome::Completed);
    }
    if project.scheduler.has_failed() {
        project.state = PipelineState::Failed;
        return Ok(PipelineOutcome::Failed("quality gate failed".into()));
    }

    Ok(PipelineOutcome::Continue)
}

pub async fn prepare_project_inputs(app: &App, project: &mut Project) -> Result<()> {
    let project_id = project.id.0;

    if let Some(pdf) = &project.pdf_bytes {
        let pdf_key = format!("projects/{project_id}/plan.pdf");
        app.artifacts
            .put(
                pdf_key.as_str(),
                Bytes::from(pdf.clone()),
                "application/pdf",
            )
            .await?;
        if project.accumulated_artifacts.is_empty() {
            project.accumulated_artifacts.push(ArtifactRef {
                name: "plan.pdf".into(),
                key: pdf_key.clone(),
                uri: app.artifacts.uri_for(&pdf_key),
                content_type: "application/pdf".into(),
                sha256: None,
            });
        }
        // Redis 등 영속 저장소에 대용량 바이너리를 중복 저장하지 않도록 정리
        project.pdf_bytes = None;
    }

    if let Some(devops) = &project.devops_plan {
        if devops.has_content() {
            let storage_name = devops_storage_name(devops);
            let key = format!("projects/{project_id}/{storage_name}");

            let (bytes, content_type) = if let Some(b) = &devops.bytes {
                (
                    Bytes::from(b.clone()),
                    devops
                        .content_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into()),
                )
            } else if let Some(text) = &devops.text {
                if text.trim().is_empty() {
                    return Ok(());
                }
                (Bytes::from(text.clone()), "text/markdown".into())
            } else {
                return Ok(());
            };

            app.artifacts
                .put(key.as_str(), bytes, &content_type)
                .await?;
            project.accumulated_artifacts.push(ArtifactRef {
                name: storage_name.clone(),
                key: key.clone(),
                uri: app.artifacts.uri_for(&key),
                content_type,
                sha256: None,
            });
        }
    }

    // DevOps 계획서 원본 바이트도 아티팩트 스토어에만 유지 (Project 저장 시 중복 방지)
    if let Some(devops) = project.devops_plan.as_mut() {
        devops.bytes = None;
    }

    Ok(())
}

#[allow(dead_code)]
pub async fn prepare_project_pdf(app: &App, project: &mut Project) -> Result<()> {
    prepare_project_inputs(app, project).await
}

fn devops_storage_name(devops: &crate::domain::DevopsPlanInput) -> String {
    if let Some(name) = &devops.filename {
        if name.starts_with("devops_plan") {
            return name.clone();
        }
        let ext = name.rsplit('.').next().unwrap_or("md");
        return format!("devops_plan.{ext}");
    }
    "devops_plan.md".into()
}

fn parse_architect_questions(metadata: &serde_json::Value) -> Vec<ArchitectureClarification> {
    metadata
        .get("questions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let question = item
                        .get("question")
                        .or_else(|| item.get("text"))
                        .and_then(|v| v.as_str())?
                        .to_string();
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("q")
                        .to_string();
                    let options = item
                        .get("options")
                        .and_then(|v| v.as_array())
                        .map(|opts| {
                            opts.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let required = item
                        .get("required")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let category = item
                        .get("category")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    Some(ArchitectureClarification {
                        id,
                        question,
                        options,
                        required,
                        category,
                        answer: None,
                        answered_at: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 아키텍처 Q&A 답변 제출 후 파이프라인 재개
pub async fn submit_architecture_answers(
    app: &App,
    project: &mut Project,
    answers: Vec<crate::domain::ArchitectureAnswerInput>,
) -> Result<()> {
    if project.state != PipelineState::AwaitingInput {
        return Err(AutoForgeError::BadRequest(
            "project is not awaiting architecture input".into(),
        ));
    }

    for input in answers {
        if let Some(q) = project
            .architecture_clarifications
            .iter_mut()
            .find(|q| q.id == input.id)
        {
            q.answer = Some(input.answer.trim().to_string());
            q.answered_at = Some(chrono::Utc::now());
        }
    }

    if !all_required_answered(&project.architecture_clarifications) {
        return Err(AutoForgeError::BadRequest(
            "all required architecture questions must be answered".into(),
        ));
    }

    project.scheduler.record_architect_answers_submitted();
    project
        .stages
        .insert(StageId::Architect, StageState::Queued);
    project.state = PipelineState::Running;
    Ok(())
}

pub async fn resume_project_pipeline(app: Arc<App>, project_id: Uuid) -> Result<()> {
    if app.config.message_queue_enabled() && app.queue.is_some() {
        let project = app
            .store
            .get(project_id)
            .await?
            .ok_or_else(|| AutoForgeError::NotFound(format!("project {project_id}")))?;
        let cmds = project.scheduler.ready_stages();
        if let Some(mq) = &app.queue {
            mq.enqueue_commands(&cmds).await?;
        }
        Ok(())
    } else {
        let app_clone = app.clone();
        tokio::spawn(async move {
            if let Err(e) = run_inline(app_clone, project_id).await {
                tracing::error!(%project_id, error = %e, "pipeline resume failed");
            }
        });
        Ok(())
    }
}

#[derive(Debug)]
pub enum PipelineOutcome {
    Continue,
    AwaitingInput,
    Completed,
    Failed(String),
}

/// 인라인 모드 — 단일 프로세스에서 전체 파이프라인 실행
pub async fn run_inline(app: std::sync::Arc<App>, project_id: Uuid) -> Result<()> {
    let mut project = app
        .store
        .get(project_id)
        .await?
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {project_id}")))?;

    let is_fresh = project.stages.get(&StageId::Ingest) != Some(&StageState::Completed);

    if is_fresh {
        project.state = PipelineState::Running;
        app.store.save(&project).await?;
        prepare_project_pdf(&app, &mut project).await?;
        app.store.save(&project).await?;

        if let Some(slack) = &app.slack {
            if let Ok(Some(ts)) = slack.notify_project_created(&project).await {
                project.slack_message_ts = Some(ts);
                app.store.save(&project).await?;
            }
        }

        let _ = record_daily_event(
            &app,
            &mut project,
            DailyEvent {
                event: "project_created",
                stage: None,
                message: "프로젝트 생성 및 파이프라인 시작".into(),
            },
        )
        .await;
    }

    loop {
        if project.state == PipelineState::AwaitingInput {
            app.store.save(&project).await?;
            info!(%project_id, "pipeline paused awaiting architecture input");
            return Ok(());
        }

        let commands = project.scheduler.ready_stages();
        if commands.is_empty() {
            if project.scheduler.is_pipeline_complete() {
                project.state = PipelineState::Completed;
                app.store.save(&project).await?;
                if let Some(slack) = &app.slack {
                    let _ = slack
                        .notify_pipeline_done(&project, project.slack_message_ts.as_deref())
                        .await;
                }
                info!(%project_id, "pipeline completed");
                return Ok(());
            }
            if project.scheduler.has_failed() {
                project.state = PipelineState::Failed;
                app.store.save(&project).await?;
                return Err(AutoForgeError::Orchestrator("quality gate failed".into()));
            }
            if project.scheduler.is_awaiting_architecture_input() {
                project.state = PipelineState::AwaitingInput;
                app.store.save(&project).await?;
                return Ok(());
            }
            break;
        }

        // 취소 확인: 별도 요청(API)이 상태를 Cancelled로 바꿨을 수 있으므로 재조회
        if let Ok(Some(latest)) = app.store.get(project_id).await {
            if latest.state == PipelineState::Cancelled {
                project.state = PipelineState::Cancelled;
                app.store.save(&project).await?;
                info!(%project_id, "pipeline cancelled, stopping inline execution");
                return Ok(());
            }
        }

        for cmd in commands {
            let stage = cmd.stage;
            info!(%project_id, ?stage, "running stage");
            project.stages.insert(stage, StageState::Running);
            project.scheduler.mark_running(stage);
            app.store.save(&project).await?;

            if let Some(slack) = &app.slack {
                let _ = slack
                    .notify_stage_update(
                        &project,
                        stage,
                        "running",
                        project.slack_message_ts.as_deref(),
                    )
                    .await;
            }

            let _ = record_daily_event(
                &app,
                &mut project,
                DailyEvent {
                    event: "stage_running",
                    stage: Some(stage),
                    message: format!("{} 스테이지 실행 시작", stage.as_str()),
                },
            )
            .await;

            match execute_stage(&app, &project, stage).await {
                Ok(output) => {
                    let outcome =
                        apply_stage_output_async(&app, &mut project, stage, output).await?;
                    app.store.save(&project).await?;

                    if let Some(slack) = &app.slack {
                        let status = if project.stages.get(&stage) == Some(&StageState::Failed) {
                            "failed"
                        } else {
                            "completed"
                        };
                        let _ = slack
                            .notify_stage_update(
                                &project,
                                stage,
                                status,
                                project.slack_message_ts.as_deref(),
                            )
                            .await;
                    }

                    let event_name = if project.stages.get(&stage) == Some(&StageState::Failed) {
                        "stage_failed"
                    } else {
                        "stage_completed"
                    };
                    let _ = record_daily_event(
                        &app,
                        &mut project,
                        DailyEvent {
                            event: event_name,
                            stage: Some(stage),
                            message: format!("{} 스테이지 {}", stage.as_str(), event_name),
                        },
                    )
                    .await;

                    match outcome {
                        PipelineOutcome::Completed => {
                            if let Some(slack) = &app.slack {
                                let _ = slack
                                    .notify_pipeline_done(
                                        &project,
                                        project.slack_message_ts.as_deref(),
                                    )
                                    .await;
                            }
                            let _ = record_daily_event(
                                &app,
                                &mut project,
                                DailyEvent {
                                    event: "pipeline_completed",
                                    stage: None,
                                    message: "파이프라인 완료".into(),
                                },
                            )
                            .await;
                            return Ok(());
                        }
                        PipelineOutcome::Failed(msg) => {
                            if let Some(slack) = &app.slack {
                                let _ = slack
                                    .notify_pipeline_failed(
                                        &project,
                                        &msg,
                                        project.slack_message_ts.as_deref(),
                                    )
                                    .await;
                            }
                            let _ = record_daily_event(
                                &app,
                                &mut project,
                                DailyEvent {
                                    event: "pipeline_failed",
                                    stage: Some(stage),
                                    message: msg.clone(),
                                },
                            )
                            .await;
                            return Err(AutoForgeError::StageFailed {
                                stage,
                                message: msg,
                            });
                        }
                        PipelineOutcome::AwaitingInput => {
                            let _ = record_daily_event(
                                &app,
                                &mut project,
                                DailyEvent {
                                    event: "architecture_input_required",
                                    stage: Some(stage),
                                    message: "아키텍처 설계 질문에 대한 답변 필요".into(),
                                },
                            )
                            .await;
                            app.store.save(&project).await?;
                            info!(%project_id, "pipeline paused for architecture Q&A");
                            return Ok(());
                        }
                        PipelineOutcome::Continue => {}
                    }
                }
                Err(e) => {
                    warn!(%project_id, ?stage, error = %e, "stage failed");
                    project.stages.insert(stage, StageState::Failed);
                    project.scheduler.mark_failed(stage);
                    project.state = PipelineState::Failed;
                    app.store.save(&project).await?;
                    if let Some(slack) = &app.slack {
                        let _ = slack
                            .notify_pipeline_failed(
                                &project,
                                &e.to_string(),
                                project.slack_message_ts.as_deref(),
                            )
                            .await;
                    }
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}
