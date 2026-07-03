use crate::app::App;
use crate::domain::{
    DailyLogSummary, DevopsPlanInput, PipelineState, ProjectDetailView, ProjectView, StageState,
};
use crate::error::{AutoForgeError, Result};
use crate::services::artifacts::{
    detect_image_extension, guess_image_content_type, ArtifactStore, MEDIA_DIR,
};
use crate::services::github::ensure_project_repo;
use crate::services::health::{self, HealthReport};
use crate::services::pipeline::{run_inline, start_project_mq};
use actix_multipart::Multipart;
use actix_web::http::header;
use actix_web::{web, HttpResponse};
use futures_util::{StreamExt, TryStreamExt};
use std::sync::Arc;
use uuid::Uuid;

/// Liveness — 프로세스 생존 확인 (의존성 프로브 없음)
pub async fn health(app: web::Data<Arc<App>>) -> HttpResponse {
    let mut report = HealthReport::liveness();
    report.message_queue = app.queue.is_some();
    report.worker_concurrency = app.config.worker_concurrency;
    report.github_auto_merge = app.config.github_auto_merge;
    HttpResponse::Ok().json(serde_json::json!({
        "status": report.status,
        "service": report.service,
        "message_queue": report.message_queue,
        "worker_concurrency": report.worker_concurrency,
        "github_auto_merge": report.github_auto_merge,
        "auth_enabled": app.config.auth_enabled(),
    }))
}

/// Readiness — 스토어/큐/AI API 연결 확인
pub async fn ready(app: web::Data<Arc<App>>) -> HttpResponse {
    let report = health::readiness(app.get_ref()).await;
    if report.status == "unhealthy" {
        HttpResponse::ServiceUnavailable().json(report)
    } else {
        HttpResponse::Ok().json(report)
    }
}

pub async fn list_projects(app: web::Data<Arc<App>>) -> Result<HttpResponse> {
    let projects = app.store.list().await?;
    let views: Vec<ProjectView> = projects.iter().map(ProjectView::from).collect();
    Ok(HttpResponse::Ok().json(views))
}

pub async fn get_project(app: web::Data<Arc<App>>, path: web::Path<Uuid>) -> Result<HttpResponse> {
    let id = path.into_inner();
    let project = app
        .get_project(id)
        .await
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {id}")))?;
    Ok(HttpResponse::Ok().json(ProjectDetailView::from(&project)))
}

pub async fn create_project(
    app: web::Data<Arc<App>>,
    mut payload: Multipart,
) -> Result<HttpResponse> {
    let mut name: Option<String> = None;
    let mut repo_url: Option<String> = None;
    let mut pdf_bytes: Option<Vec<u8>> = None;
    let mut devops_plan = DevopsPlanInput::default();
    let max_upload = app.config.max_upload_bytes;
    let mut total_bytes: usize = 0;

    while let Some(field) = payload
        .try_next()
        .await
        .map_err(|e| AutoForgeError::BadRequest(e.to_string()))?
    {
        let field_name = field
            .content_disposition()
            .and_then(|d| d.get_name().map(String::from))
            .unwrap_or_default();

        let field_filename = field
            .content_disposition()
            .and_then(|d| d.get_filename().map(String::from));
        let field_content_type = field.content_type().map(|m| m.to_string());

        let mut data = Vec::new();
        let mut field = field;
        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|e| AutoForgeError::BadRequest(e.to_string()))?;
            total_bytes += chunk.len();
            if total_bytes > max_upload {
                return Err(AutoForgeError::BadRequest(format!(
                    "upload too large (limit: {max_upload} bytes)"
                )));
            }
            data.extend_from_slice(&chunk);
        }

        match field_name.as_str() {
            "name" => {
                let value = String::from_utf8_lossy(&data).trim().to_string();
                if value.len() > 200 {
                    return Err(AutoForgeError::BadRequest(
                        "name too long (max 200 chars)".into(),
                    ));
                }
                if !value.is_empty() {
                    name = Some(value);
                }
            }
            "repo_url" => {
                let value = String::from_utf8_lossy(&data).trim().to_string();
                if !value.is_empty() {
                    if !(value.starts_with("https://github.com/")
                        || value.starts_with("git@github.com:"))
                    {
                        return Err(AutoForgeError::BadRequest(
                            "repo_url must be a github.com HTTPS or SSH URL".into(),
                        ));
                    }
                    repo_url = Some(value);
                }
            }
            "devops_plan_text" | "devops_text" => {
                devops_plan.text = Some(String::from_utf8_lossy(&data).to_string());
            }
            "devops_plan" | "devops" | "devops_file" => {
                devops_plan.bytes = Some(data);
                devops_plan.filename = field_filename;
                devops_plan.content_type = field_content_type;
            }
            "plan" | "pdf" | "file" => pdf_bytes = Some(data),
            _ => {}
        }
    }

    let pdf = pdf_bytes
        .ok_or_else(|| AutoForgeError::BadRequest("PDF file required (field: plan)".into()))?;

    if pdf.is_empty() {
        return Err(AutoForgeError::BadRequest("empty PDF file".into()));
    }

    if !pdf.starts_with(b"%PDF") {
        return Err(AutoForgeError::BadRequest("invalid PDF file".into()));
    }

    let mut project = app.create_project(name, repo_url).await;
    project.pdf_bytes = Some(pdf);
    if devops_plan.has_content() {
        project.devops_plan = Some(devops_plan);
    }

    // GitHub 프라이빗 레포 자동 생성 (repo_url 미지정 시)
    if project.repo_url.is_none() {
        ensure_project_repo(&app, &mut project).await?;
        if project.repo_url.is_none() {
            project.repo_url = app.config.default_repo_url.clone();
        }
    }

    project.state = PipelineState::Running;
    let project_id = project.id.0;
    app.store.save(&project).await?;

    if app.config.message_queue_enabled() && app.queue.is_some() {
        start_project_mq(app.get_ref().clone(), project_id).await?;
    } else {
        let app_clone = app.get_ref().clone();
        tokio::spawn(async move {
            if let Err(e) = run_inline(app_clone, project_id).await {
                tracing::error!(%project_id, error = %e, "pipeline failed");
            }
        });
    }

    let project = app
        .get_project(project_id)
        .await
        .ok_or_else(|| AutoForgeError::Internal("project lost after create".into()))?;

    Ok(HttpResponse::Accepted().json(serde_json::json!({
        "id": project_id,
        "state": project.state,
        "repo_url": project.repo_url,
        "message": if app.queue.is_some() { "pipeline queued" } else { "pipeline started" },
        "mode": if app.queue.is_some() { "message_queue" } else { "inline" },
        "stream_url": format!("/v1/projects/{project_id}/stream"),
        "progress_percent": project.progress_percent(),
        "github_auto_created": project.stage_outputs.get(&crate::domain::StageId::Ingest)
            .and_then(|m| m.get("auto_created"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "has_devops_plan": project.devops_plan.as_ref().is_some_and(|d| d.has_content()),
    })))
}

pub async fn stream_project(
    app: web::Data<Arc<App>>,
    path: web::Path<Uuid>,
) -> Result<HttpResponse> {
    let id = path.into_inner();
    let project = app
        .get_project(id)
        .await
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {id}")))?;

    let stages: Vec<_> = crate::domain::StageId::all()
        .iter()
        .map(|stage| {
            let status = project
                .stages
                .get(stage)
                .copied()
                .unwrap_or(StageState::Queued);
            serde_json::json!({
                "stage": stage.as_str(),
                "status": format!("{status:?}").to_lowercase(),
            })
        })
        .collect();

    let body = format!(
        "event: status\ndata: {}\n\n",
        serde_json::json!({
            "project_id": id,
            "state": project.state,
            "progress_percent": project.progress_percent(),
            "stages": stages,
        })
    );

    Ok(HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .body(body))
}

pub async fn cancel_project(
    app: web::Data<Arc<App>>,
    path: web::Path<Uuid>,
) -> Result<HttpResponse> {
    let id = path.into_inner();
    let mut project = app
        .get_project(id)
        .await
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {id}")))?;

    project.state = PipelineState::Cancelled;
    app.store.save(&project).await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "id": id,
        "state": "cancelled",
    })))
}

pub async fn list_daily_logs(
    app: web::Data<Arc<App>>,
    path: web::Path<Uuid>,
) -> Result<HttpResponse> {
    let id = path.into_inner();
    let project = app
        .get_project(id)
        .await
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {id}")))?;

    let mut logs: Vec<DailyLogSummary> = project
        .daily_logs
        .values()
        .map(DailyLogSummary::from)
        .collect();
    logs.sort_by(|a, b| a.date.cmp(&b.date));

    Ok(HttpResponse::Ok().json(logs))
}

pub async fn get_daily_log(
    app: web::Data<Arc<App>>,
    path: web::Path<(Uuid, String)>,
) -> Result<HttpResponse> {
    let (id, date) = path.into_inner();
    let project = app
        .get_project(id)
        .await
        .ok_or_else(|| AutoForgeError::NotFound(format!("project {id}")))?;

    let log = project
        .daily_logs
        .get(&date)
        .cloned()
        .ok_or_else(|| AutoForgeError::NotFound(format!("daily log {date}")))?;

    Ok(HttpResponse::Ok().json(log))
}

/// 이미지 업로드 (multipart, 필드명: `image` 또는 `file`).
/// 매직 바이트로 이미지 형식을 확인하고 `media/{uuid}.{ext}`로 저장한 뒤
/// 바로 접근 가능한 공개 URL을 반환한다.
pub async fn upload_image(
    app: web::Data<Arc<App>>,
    mut payload: Multipart,
) -> Result<HttpResponse> {
    let max_bytes = app.config.max_image_bytes;

    while let Some(field) = payload
        .try_next()
        .await
        .map_err(|e| AutoForgeError::BadRequest(e.to_string()))?
    {
        let field_name = field
            .content_disposition()
            .and_then(|d| d.get_name().map(String::from))
            .unwrap_or_default();

        if field_name != "image" && field_name != "file" {
            continue;
        }

        let mut data = Vec::new();
        let mut field = field;
        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|e| AutoForgeError::BadRequest(e.to_string()))?;
            if data.len() + chunk.len() > max_bytes {
                return Err(AutoForgeError::BadRequest(format!(
                    "image too large (limit: {max_bytes} bytes)"
                )));
            }
            data.extend_from_slice(&chunk);
        }

        let ext = detect_image_extension(&data).ok_or_else(|| {
            AutoForgeError::BadRequest(
                "unsupported or invalid image (png/jpg/gif/webp/bmp/svg만 지원)".into(),
            )
        })?;

        let filename = format!("{}.{ext}", Uuid::new_v4());
        let key = format!("{MEDIA_DIR}/{filename}");
        let content_type = guess_image_content_type(&filename);
        let artifact = app.media.put(&key, data.into(), content_type).await?;

        return Ok(HttpResponse::Created().json(serde_json::json!({
            "filename": filename,
            "url": artifact.uri,
            "content_type": content_type,
        })));
    }

    Err(AutoForgeError::BadRequest(
        "image file required (field: image)".into(),
    ))
}

/// 업로드된 이미지 목록 (최신순)
pub async fn list_images(app: web::Data<Arc<App>>) -> Result<HttpResponse> {
    let images = app.media.list_media().await?;
    Ok(HttpResponse::Ok().json(images))
}

/// 업로드된 이미지를 직접 서빙한다 (인증 불필요 — 외부 공유/임베드 목적).
pub async fn serve_media(
    path: web::Path<String>,
    app: web::Data<Arc<App>>,
) -> Result<HttpResponse> {
    let filename = path.into_inner();
    if filename.contains('/') || filename.contains("..") {
        return Err(AutoForgeError::BadRequest("invalid filename".into()));
    }
    let key = format!("{MEDIA_DIR}/{filename}");
    let bytes = app
        .artifacts
        .get(&key)
        .await
        .map_err(|_| AutoForgeError::NotFound(format!("image {filename}")))?;
    let content_type = guess_image_content_type(&filename);

    Ok(HttpResponse::Ok()
        .content_type(content_type)
        .insert_header((header::CACHE_CONTROL, "public, max-age=31536000, immutable"))
        .body(bytes))
}
