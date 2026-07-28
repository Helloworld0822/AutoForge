use crate::error::{AutoForgeError, Result};
use futures_util::StreamExt;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tokio::sync::broadcast;
use uuid::Uuid;

const PROJECT_CHANNEL_PREFIX: &str = "autoforge:watch:project:";
const ALL_CHANNEL: &str = "autoforge:watch:all";

#[derive(Clone)]
pub enum WatchEvent {
    Project(Uuid),
}

#[derive(Clone)]
enum WatchBackend {
    Redis { redis_url: String },
    Memory { tx: broadcast::Sender<WatchEvent> },
}

/// 프로젝트 저장 시 WebSocket 클라이언트에 푸시 알림을 보낸다.
#[derive(Clone)]
pub struct ProjectWatch {
    backend: WatchBackend,
}

impl ProjectWatch {
    pub fn memory() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            backend: WatchBackend::Memory { tx },
        }
    }

    pub fn redis(redis_url: impl Into<String>) -> Self {
        Self {
            backend: WatchBackend::Redis {
                redis_url: redis_url.into(),
            },
        }
    }

    pub fn notify(&self, project_id: Uuid) {
        match &self.backend {
            WatchBackend::Memory { tx } => {
                let _ = tx.send(WatchEvent::Project(project_id));
            }
            WatchBackend::Redis { redis_url } => {
                let redis_url = redis_url.clone();
                tokio::spawn(async move {
                    if let Err(e) = publish_redis(&redis_url, project_id).await {
                        tracing::warn!(%project_id, error = %e, "project watch publish failed");
                    }
                });
            }
        }
    }

    pub async fn subscribe_all(&self) -> Result<WatchSubscription> {
        match &self.backend {
            WatchBackend::Memory { tx } => {
                let rx = tx.subscribe();
                Ok(WatchSubscription::Memory {
                    rx,
                    project_id: Uuid::nil(),
                })
            }
            WatchBackend::Redis { redis_url } => {
                let client = redis::Client::open(redis_url.as_str())
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                let mut pubsub = client
                    .get_async_pubsub()
                    .await
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                pubsub
                    .subscribe(ALL_CHANNEL)
                    .await
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                Ok(WatchSubscription::Redis {
                    pubsub,
                    project_id: Uuid::nil(),
                })
            }
        }
    }

    pub async fn subscribe(&self, project_id: Uuid) -> Result<WatchSubscription> {
        match &self.backend {
            WatchBackend::Memory { tx } => {
                let rx = tx.subscribe();
                Ok(WatchSubscription::Memory { rx, project_id })
            }
            WatchBackend::Redis { redis_url } => {
                let client = redis::Client::open(redis_url.as_str())
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                let mut pubsub = client
                    .get_async_pubsub()
                    .await
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                pubsub
                    .subscribe(format!("{PROJECT_CHANNEL_PREFIX}{project_id}"))
                    .await
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                pubsub
                    .subscribe(ALL_CHANNEL)
                    .await
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                Ok(WatchSubscription::Redis { pubsub, project_id })
            }
        }
    }
}

async fn publish_redis(redis_url: &str, project_id: Uuid) -> Result<()> {
    let client =
        redis::Client::open(redis_url).map_err(|e| AutoForgeError::Queue(e.to_string()))?;
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
    let id = project_id.to_string();
    redis::pipe()
        .publish(format!("{PROJECT_CHANNEL_PREFIX}{project_id}"), &id)
        .publish(ALL_CHANNEL, &id)
        .query_async::<()>(&mut conn)
        .await
        .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
    Ok(())
}

pub enum WatchSubscription {
    Memory {
        rx: broadcast::Receiver<WatchEvent>,
        project_id: Uuid,
    },
    Redis {
        pubsub: redis::aio::PubSub,
        project_id: Uuid,
    },
}

impl WatchSubscription {
    pub async fn next_for(&mut self, project_id: Uuid) -> Option<()> {
        match self {
            WatchSubscription::Memory { rx, .. } => loop {
                match rx.recv().await {
                    Ok(WatchEvent::Project(id)) if id == project_id => return Some(()),
                    Ok(WatchEvent::Project(_)) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            },
            WatchSubscription::Redis { pubsub, .. } => loop {
                let msg = pubsub.on_message().next().await?;
                let payload: String = msg.get_payload().ok()?;
                if payload == project_id.to_string() {
                    return Some(());
                }
            },
        }
    }

    pub async fn next_any(&mut self) -> Option<Uuid> {
        match self {
            WatchSubscription::Memory { rx, .. } => loop {
                match rx.recv().await {
                    Ok(WatchEvent::Project(id)) => return Some(id),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            },
            WatchSubscription::Redis { pubsub, .. } => {
                let msg = pubsub.on_message().next().await?;
                let payload: String = msg.get_payload().ok()?;
                Uuid::parse_str(&payload).ok()
            }
        }
    }
}

pub fn hash_json(value: &serde_json::Value) -> u64 {
    let json = value.to_string();
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    hasher.finish()
}
