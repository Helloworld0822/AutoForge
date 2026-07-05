use crate::app::App;
use crate::domain::{PipelineState, StageState};
use crate::error::{AutoForgeError, Result};
use crate::services::daily_log::DailyEvent;
use crate::services::daily_log_notify::record_daily_event;
use crate::services::github::ensure_project_repo;
use crate::services::pipeline::engine::{
    apply_stage_output_async, execute_stage, prepare_project_pdf, PipelineOutcome,
};
use crate::services::queue::messages::PipelineEvent;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// MQ 모드: 프로젝트 생성 후 이벤트 발행
pub async fn start_project_mq(app: Arc<App>, project_id: Uuid) -> Result<()> {
    let mut project = app
        .store
        .get(project_id)
        .await?
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {project_id}")))?;

    project.state = PipelineState::Running;
    ensure_project_repo(&app, &mut project).await?;
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

    let mq = app
        .queue
        .as_ref()
        .ok_or_else(|| AutoForgeError::Queue("message queue not configured".into()))?;

    mq.publish_event(&PipelineEvent::ProjectCreated {
        project_id,
        name: project.name.clone(),
    })
    .await?;

    info!(%project_id, "project queued via message queue");
    Ok(())
}

/// Worker 루프 — Redis Streams 커맨드 소비
pub async fn run_worker(
    app: Arc<App>,
    consumer_name: String,
    stage_filter: Option<String>,
) -> Result<()> {
    let mq = app
        .queue
        .as_ref()
        .ok_or_else(|| AutoForgeError::Queue("message queue not configured".into()))?;

    info!(%consumer_name, ?stage_filter, "worker started");

    loop {
        let mut messages = mq.read_commands(&consumer_name, 1, 5000).await?;
        // 이전 워커 크래시로 오래 pending 상태인 커맨드를 재소유 (최대 5분 idle)
        if let Ok(stale) = mq.claim_stale_commands(&consumer_name, 300_000, 1).await {
            messages.extend(stale);
        }
        for (msg_id, cmd) in messages {
            let stage = cmd.stage;
            if let Some(ref filter) = stage_filter {
                if stage.as_str() != filter {
                    mq.ack_command(&msg_id).await?;
                    continue;
                }
            }

            let project_id = cmd.project_id;
            let mut project = match app.store.get(project_id).await? {
                Some(p) => p,
                None => {
                    warn!(%project_id, "project not found, acking command");
                    mq.ack_command(&msg_id).await?;
                    continue;
                }
            };

            if project.state == PipelineState::Cancelled {
                info!(%project_id, ?stage, "project cancelled, skipping command");
                mq.ack_command(&msg_id).await?;
                continue;
            }

            if project.stages.get(&stage) == Some(&StageState::Completed) {
                warn!(%project_id, ?stage, "stage already completed, skipping duplicate command (idempotency)");
                mq.ack_command(&msg_id).await?;
                continue;
            }

            info!(%project_id, ?stage, "worker executing stage");
            project.stages.insert(stage, StageState::Running);
            project.scheduler.mark_running(stage);
            app.store.save(&project).await?;

            mq.publish_event(&PipelineEvent::StageStarted { project_id, stage })
                .await?;

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
                    let passed = output.metadata.get("passed").and_then(|v| v.as_bool());
                    mq.publish_event(&PipelineEvent::StageCompleted {
                        project_id,
                        stage,
                        metadata: output.metadata.clone(),
                        artifacts: output.artifacts.clone(),
                        passed,
                    })
                    .await?;
                }
                Err(e) => {
                    mq.publish_event(&PipelineEvent::StageFailed {
                        project_id,
                        stage,
                        error: e.to_string(),
                    })
                    .await?;
                }
            }

            mq.ack_command(&msg_id).await?;
        }
    }
}

/// 이벤트 처리 실패 시 최대 재시도 횟수 (초과 시 DLQ로 이동)
const MAX_EVENT_RETRIES: i64 = 5;
/// 재시도(claim) 대상으로 간주할 최소 idle 시간 (ms) — 컨슈머 크래시/장기 실패 복구용
const STALE_CLAIM_MIN_IDLE_MS: i64 = 30_000;

/// Orchestrator 루프 — 이벤트 소비 후 다음 커맨드 enqueue
///
/// 처리에 성공한 이벤트만 ACK한다. 실패한 이벤트는 pending 상태로 남아
/// 주기적으로 재시도(claim)되며, 최대 재시도 횟수를 초과하면 데드레터 스트림으로
/// 옮기고 ACK하여 무한 재처리를 방지한다.
pub async fn run_orchestrator(app: Arc<App>, consumer_name: String) -> Result<()> {
    let mq = app
        .queue
        .as_ref()
        .ok_or_else(|| AutoForgeError::Queue("message queue not configured".into()))?;

    info!(%consumer_name, "orchestrator started");

    loop {
        let messages = mq.read_events(&consumer_name, 10, 5000).await?;
        for (msg_id, event) in messages {
            handle_event_with_retry(&app, mq, &msg_id, &event).await;
        }

        // 컨슈머 크래시나 앞선 실패로 오래 pending 상태인 이벤트를 재소유하여 재시도
        match mq
            .claim_stale_events(&consumer_name, STALE_CLAIM_MIN_IDLE_MS, 10)
            .await
        {
            Ok(stale) => {
                for (msg_id, event) in stale {
                    handle_event_with_retry(&app, mq, &msg_id, &event).await;
                }
            }
            Err(e) => warn!(error = %e, "failed to claim stale events"),
        }
    }
}

async fn handle_event_with_retry(
    app: &App,
    mq: &crate::services::queue::MessageQueue,
    msg_id: &str,
    event: &PipelineEvent,
) {
    match handle_event(app, event).await {
        Ok(()) => {
            if let Err(e) = mq.ack_event(msg_id).await {
                error!(error = %e, msg_id, "failed to ack event after successful handling");
            }
        }
        Err(e) => {
            let attempt = mq.incr_retry(&mq.events_stream, msg_id).await.unwrap_or(1);
            error!(?event, error = %e, attempt, "orchestrator event handling failed");

            if attempt >= MAX_EVENT_RETRIES {
                let payload = serde_json::to_string(event).unwrap_or_default();
                if let Err(dlq_err) = mq
                    .dead_letter(&mq.events_stream, msg_id, &payload, &e.to_string())
                    .await
                {
                    error!(error = %dlq_err, "failed to move event to dead-letter stream");
                } else {
                    error!(
                        ?event,
                        attempt, "event moved to dead-letter stream after max retries"
                    );
                }
                if let Err(ack_err) = mq.ack_event(msg_id).await {
                    error!(error = %ack_err, "failed to ack dead-lettered event");
                }
            }
            // 재시도 한도 이내라면 ACK하지 않고 pending 상태로 남겨 다음 claim 주기에 재시도
        }
    }
}

async fn handle_event(app: &App, event: &PipelineEvent) -> Result<()> {
    let mq = app
        .queue
        .as_ref()
        .ok_or_else(|| AutoForgeError::Queue("message queue not configured".into()))?;

    match event {
        PipelineEvent::ProjectCreated { project_id, .. } => {
            let project = app
                .store
                .get(*project_id)
                .await?
                .ok_or_else(|| AutoForgeError::NotFound(format!("project {project_id}")))?;
            let cmds = project.scheduler.ready_stages();
            mq.enqueue_commands(&cmds).await?;
            info!(%project_id, count = cmds.len(), "initial commands enqueued");
        }
        PipelineEvent::StageStarted { .. } => {}
        PipelineEvent::StageCompleted {
            project_id,
            stage,
            metadata,
            artifacts,
            passed,
        } => {
            let mut project = app
                .store
                .get(*project_id)
                .await?
                .ok_or_else(|| AutoForgeError::NotFound(format!("project {project_id}")))?;

            // 멱등성: 이미 완료 처리된 스테이지 이벤트는 무시 (중복 전달 방지)
            if project.stages.get(stage) == Some(&StageState::Completed) {
                info!(%project_id, ?stage, "stage already applied, ignoring duplicate event");
                return Ok(());
            }

            let output = crate::services::worker::StageOutput {
                artifacts: artifacts.clone(),
                metadata: metadata.clone(),
            };
            let outcome = apply_stage_output_async(app, &mut project, *stage, output).await?;
            app.store.save(&project).await?;

            if let Some(slack) = &app.slack {
                let status = if *passed == Some(false) && *stage == crate::domain::StageId::Verify {
                    "failed (will debug)"
                } else {
                    "completed"
                };
                let _ = slack
                    .notify_stage_update(
                        &project,
                        *stage,
                        status,
                        project.slack_message_ts.as_deref(),
                    )
                    .await;
            }

            let event_name = if project.stages.get(stage) == Some(&StageState::Failed) {
                "stage_failed"
            } else {
                "stage_completed"
            };
            let _ = record_daily_event(
                app,
                &mut project,
                DailyEvent {
                    event: event_name,
                    stage: Some(*stage),
                    message: format!("{} 스테이지 {}", stage.as_str(), event_name),
                },
            )
            .await;

            match outcome {
                PipelineOutcome::Completed => {
                    mq.publish_event(&PipelineEvent::PipelineCompleted {
                        project_id: *project_id,
                    })
                    .await?;
                    if let Some(slack) = &app.slack {
                        let _ = slack
                            .notify_pipeline_done(&project, project.slack_message_ts.as_deref())
                            .await;
                    }
                    let _ = record_daily_event(
                        app,
                        &mut project,
                        DailyEvent {
                            event: "pipeline_completed",
                            stage: None,
                            message: "파이프라인 완료".into(),
                        },
                    )
                    .await;
                }
                PipelineOutcome::Failed(msg) => {
                    mq.publish_event(&PipelineEvent::PipelineFailed {
                        project_id: *project_id,
                        error: msg.clone(),
                    })
                    .await?;
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
                        app,
                        &mut project,
                        DailyEvent {
                            event: "pipeline_failed",
                            stage: Some(*stage),
                            message: msg.clone(),
                        },
                    )
                    .await;
                }
                PipelineOutcome::AwaitingInput => {
                    let _ = record_daily_event(
                        app,
                        &mut project,
                        DailyEvent {
                            event: "architecture_input_required",
                            stage: Some(*stage),
                            message: "아키텍처 설계 질문에 대한 답변 필요".into(),
                        },
                    )
                    .await;
                }
                PipelineOutcome::Continue => {
                    if project.state == PipelineState::Cancelled {
                        info!(%project_id, "project cancelled, not enqueueing further stages");
                    } else {
                        let cmds = project.scheduler.ready_stages();
                        mq.enqueue_commands(&cmds).await?;
                    }
                }
            }
        }
        PipelineEvent::StageFailed {
            project_id,
            stage,
            error,
        } => {
            let mut project = app
                .store
                .get(*project_id)
                .await?
                .ok_or_else(|| AutoForgeError::NotFound(format!("project {project_id}")))?;
            project.stages.insert(*stage, StageState::Failed);
            project.scheduler.mark_failed(*stage);
            project.state = PipelineState::Failed;
            app.store.save(&project).await?;
            if let Some(slack) = &app.slack {
                let _ = slack
                    .notify_pipeline_failed(&project, error, project.slack_message_ts.as_deref())
                    .await;
            }
            let _ = record_daily_event(
                app,
                &mut project,
                DailyEvent {
                    event: "pipeline_failed",
                    stage: Some(*stage),
                    message: error.clone(),
                },
            )
            .await;
        }
        PipelineEvent::PipelineCompleted { .. } | PipelineEvent::PipelineFailed { .. } => {}
    }
    Ok(())
}
