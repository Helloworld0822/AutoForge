use crate::app::App;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub status: &'static str,
    pub service: &'static str,
    pub checks: BTreeMap<String, CheckResult>,
    pub message_queue: bool,
    pub worker_concurrency: usize,
    pub github_auto_merge: bool,
}

impl HealthReport {
    pub fn liveness() -> Self {
        Self {
            status: "ok",
            service: "autoforge",
            checks: BTreeMap::new(),
            message_queue: false,
            worker_concurrency: 0,
            github_auto_merge: false,
        }
    }
}

async fn timed_check<F, Fut>(name: &str, f: F) -> (String, CheckResult)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let start = Instant::now();
    match f().await {
        Ok(()) => (
            name.to_string(),
            CheckResult {
                status: "ok",
                message: None,
                latency_ms: Some(start.elapsed().as_millis() as u64),
            },
        ),
        Err(message) => (
            name.to_string(),
            CheckResult {
                status: "error",
                message: Some(message),
                latency_ms: Some(start.elapsed().as_millis() as u64),
            },
        ),
    }
}

fn skipped(message: impl Into<String>) -> CheckResult {
    CheckResult {
        status: "skipped",
        message: Some(message.into()),
        latency_ms: None,
    }
}

/// 외부 의존성(Redis, Cursor, Stitch) 및 스토어 프로브 — readiness 용
pub async fn readiness(app: &App) -> HealthReport {
    let mut checks = BTreeMap::new();
    let mut unhealthy = false;

    let store_ok = app.store.list().await.is_ok();
    checks.insert(
        "store".to_string(),
        CheckResult {
            status: if store_ok { "ok" } else { "error" },
            message: if store_ok {
                None
            } else {
                Some("project store unavailable".into())
            },
            latency_ms: None,
        },
    );
    if !store_ok {
        unhealthy = true;
    }

    let artifacts_durable = app.artifacts.is_durable();
    checks.insert(
        "artifacts_durable".to_string(),
        CheckResult {
            status: if artifacts_durable { "ok" } else { "skipped" },
            message: if artifacts_durable {
                None
            } else {
                Some("in-memory artifact store (dev only)".into())
            },
            latency_ms: None,
        },
    );

    if let Some(mq) = &app.queue {
        let mq = mq.clone();
        let (k, v) = timed_check("redis", || {
            let mq = mq.clone();
            async move { mq.ping().await.map_err(|e| e.to_string()) }
        })
        .await;
        if v.status == "error" {
            unhealthy = true;
        }
        checks.insert(k, v);
    } else if app.config.message_queue_enabled() {
        let (k, v) = (
            "redis".to_string(),
            CheckResult {
                status: "error",
                message: Some("message queue enabled but not connected".into()),
                latency_ms: None,
            },
        );
        unhealthy = true;
        checks.insert(k, v);
    } else {
        checks.insert("redis".into(), skipped("inline mode"));
    }

    if app.config.cursor_api_key.is_empty() {
        checks.insert(
            "cursor_api".into(),
            skipped("CURSOR_API_KEY not configured"),
        );
    } else {
        let cursor = app.cursor.clone();
        let (k, v) = timed_check("cursor_api", || {
            let cursor = cursor.clone();
            async move { cursor.health_check().await }
        })
        .await;
        if v.status == "error" {
            unhealthy = true;
        }
        checks.insert(k, v);
    }

    if app.config.stitch_api_key.is_empty() {
        checks.insert(
            "stitch_api".into(),
            skipped("STITCH_API_KEY not configured"),
        );
    } else {
        let stitch = app.stitch.clone();
        let (k, v) = timed_check("stitch_api", || {
            let stitch = stitch.clone();
            async move { stitch.health_check().await }
        })
        .await;
        if v.status == "error" {
            unhealthy = true;
        }
        checks.insert(k, v);
    }

    HealthReport {
        status: if unhealthy { "unhealthy" } else { "ok" },
        service: "autoforge",
        checks,
        message_queue: app.queue.is_some(),
        worker_concurrency: app.config.worker_concurrency,
        github_auto_merge: app.config.github_auto_merge,
    }
}
