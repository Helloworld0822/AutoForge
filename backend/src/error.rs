use crate::domain::StageId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AutoForgeError {
    #[error("cursor API error: {0}")]
    CursorApi(String),

    #[error("stitch API error: {0}")]
    StitchApi(String),

    #[error("ingest error: {0}")]
    Ingest(String),

    #[error("artifact store error: {0}")]
    Artifacts(String),

    #[error("orchestration error: {0}")]
    Orchestrator(String),

    #[error("stage {stage:?} failed: {message}")]
    StageFailed { stage: StageId, message: String },

    #[error("invalid state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: crate::domain::PipelineState,
        to: crate::domain::PipelineState,
    },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("message queue error: {0}")]
    Queue(String),

    #[error("slack error: {0}")]
    Slack(String),

    #[error("github error: {0}")]
    GitHub(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, AutoForgeError>;

impl actix_web::ResponseError for AutoForgeError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        use actix_web::http::StatusCode;
        match self {
            AutoForgeError::NotFound(_) => StatusCode::NOT_FOUND,
            AutoForgeError::BadRequest(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> actix_web::HttpResponse {
        actix_web::HttpResponse::build(self.status_code()).json(serde_json::json!({
            "error": self.to_string(),
        }))
    }
}
