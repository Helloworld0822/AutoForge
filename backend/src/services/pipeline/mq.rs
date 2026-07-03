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
use crate::services::queue::MessageQueue;
use crate::shutdown;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::{error, info, warn};
use uuid::Uuid;

const REDIS_READ_RETRIES: u32 = 3;
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_EVENT_RETRIES: i64 = 5;
const STALE_CLAIM_MIN_IDLE_MS: i64 = 30_000;
const STALE_COMMAND_MIN_IDLE_MS: i64 = 300_000;

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

fn spawn_shutdown_listener(shutdown_tx: watch::Sender<bool>) {
    tokio::spawn(async move {
        shutdown::wait_for_shutdown().await;
        info!("shutdown signal received");
        let _ = shutdown_tx.send(true);
    });
}

async fn read_commands_resilient(
    mq: &MessageQueue,
    consumer: &str,
    count: usize,
    block_ms: u64,
) -> Result<Vec<(String, crate::services::queue::messages::QueueCommand)>> {
    let mut last_err = None;
    for attempt in 0..REDIS_READ_RETRIES {
        match mq.read_commands(consumer, count, block_ms).await {
            Ok(messages) => return Ok(messages),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < REDIS_READ_RETRIES {
                    warn!(attempt = attempt + 1, "redis command read failed, retrying");
                    tokio::time::sleep(Duration::from_millis(250 * (attempt + 1) as u64)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AutoForgeError::Queue("redis read failed".into())))
}

async fn read_events_resilient(
    mq: &MessageQueue,
    consumer: &str,
    count: usize,
    block_ms: u64,
) -> Result<Vec<(String, PipelineEvent)>> {
    let mut last_err = None;
    for attempt in 0..REDIS_READ_RETRIES {
        match mq.read_events(consumer, count, block_ms).await {
            Ok(messages) => return Ok(messages),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < REDIS_READ_RETRIES {
                    warn!(attempt = attempt + 1, "redis event read failed, retrying");
                    tokio::time::sleep(Duration::from_millis(250 * (attempt + 1) as u64)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AutoForgeError::Queue("redis read failed".into())))
}

/// Worker 루프 — Redis Streams 커맨드 소비 (프로세스 내 WORKER_CONCURRENCY 병렬 처리)
pub async fn run_worker(
    app: Arc<App>,
    consumer_name: String,
    stage_filter: Option<String>,
) -> Result<()> {
    let mq = app
        .queue
        .as_ref()
        .ok_or_else(|| AutoForgeError::Queue("message queue not configured".into()))?
        .clone();

    let concurrency = app.config.worker_concurrency.max(1);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    spawn_shutdown_listener(shutdown_tx);

    info!(
        %consumer_name,
        ?stage_filter,
        concurrency,
        "worker started"
    );

    let mut in_flight = JoinSet::new();

    loop {
        if *shutdown_rx.borrow() && in_flight.is_empty() {
            break;
        }

        while let Some(joined) = in_flight.try_join_next() {
            if let Err(e) = joined {
                error!(error = %e, "worker task join failed");
            }
        }

        if *shutdown_rx.borrow() {
            if in_flight.is_empty() {
                break;
            }
            if tokio::time::timeout(SHUTDOWN_DRAIN_TIMEOUT, async {
                while let Some(joined) = in_flight.join_next().await {
                    if let Err(e) = joined {
                        error!(error = %e, "worker task join failed");
                    }
                }
            })
            .await
            .is_err()
            {
                warn!("shutdown drain timed out with tasks still running");
            }
            break;
        }

        let available = concurrency.saturating_sub(in_flight.len());
        if available == 0 {
            if let Some(joined) = in_flight.join_next().await {
                if let Err(e) = joined {
                    error!(error = %e, "worker task join failed");
                }
            }
            continue;
        }

        let mut messages =
            read_commands_resilient(&mq, &consumer_name, available, 5000).await?;
        if let Ok(stale) = mq
            .claim_stale_commands(&consumer_name, STALE_COMMAND_MIN_IDLE_MS, available)
            .await
        {
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

            let app = app.clone();
            let mq = mq.clone();
            in_flight.spawn(async move {
                if let Err(e) = process_stage_command(app, mq, msg_id, cmd).await {
                    error!(?stage, error = %e, "stage command failed");
                }
            });
        }
    }

    info!(%consumer_name, "worker stopped gracefully");
    Ok(())
}

async fn process_stage_command(
    app: Arc<App>,
    mq: Arc<MessageQueue>,
    msg_id: String,
    cmd: crate::services::queue::messages::QueueCommand,
) -> Result<()> {
    let stage = cmd.stage;
    let project_id = cmd.project_id;

    let mut project = match app.store.get(project_id).await? {
        Some(p) => p,
        None => {
            warn!(%project_id, "project not found, acking command");
            mq.ack_command(&msg_id).await?;
            return Ok(());
        }
    };

    if project.state == PipelineState::Cancelled {
        info!(%project_id, ?stage, "project cancelled, skipping command");
        mq.ack_command(&msg_id).await?;
        return Ok(());
    }

    if project.stages.get(&stage) == Some(&StageState::Completed) {
        warn!(%project_id, ?stage, "stage already completed, skipping duplicate command (idempotency)");
        mq.ack_command(&msg_id).await?;
        return Ok(());
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
    Ok(())
}

/// Orchestrator 루프 — 이벤트 소비 후 다음 커맨드 enqueue
pub async fn run_orchestrator(app: Arc<App>, consumer_name: String) -> Result<()> {
    let mq = app
        .queue
        .as_ref()
        .ok_or_else(|| AutoForgeError::Queue("message queue not configured".into()))?
        .clone();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    spawn_shutdown_listener(shutdown_tx);

    info!(%consumer_name, "orchestrator started");

    loop {
        if *shutdown_rx.borrow() {
            info!(%consumer_name, "orchestrator stopped gracefully");
            break;
        }

        let messages = read_events_resilient(&mq, &consumer_name, 10, 5000).await?;
        for (msg_id, event) in messages {
            handle_event_with_retry(&app, &mq, &msg_id, &event).await;
        }

        match mq
            .claim_stale_events(&consumer_name, STALE_CLAIM_MIN_IDLE_MS, 10)
            .await
        {
            Ok(stale) => {
                for (msg_id, event) in stale {
                    handle_event_with_retry(&app, &mq, &msg_id, &event).await;
                }
            }
            Err(e) => warn!(error = %e, "failed to claim stale events"),
        }
    }

    Ok(())
}

async fn handle_event_with_retry(
    app: &App,
    mq: &MessageQueue,
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
