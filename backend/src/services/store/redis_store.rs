use super::ProjectStore;
use crate::domain::Project;
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use uuid::Uuid;

const KEY_PREFIX: &str = "autoforge:project:";

pub struct RedisProjectStore {
    conn: ConnectionManager,
}

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

impl RedisProjectStore {
    pub async fn connect(redis_url: &str) -> Result<Self> {
        let client =
            redis::Client::open(redis_url).map_err(|e| AutoForgeError::Store(e.to_string()))?;
        let conn = tokio::time::timeout(CONNECT_TIMEOUT, client.get_connection_manager())
            .await
            .map_err(|_| {
                AutoForgeError::Store(format!(
                    "timed out connecting to Redis at {redis_url} after {CONNECT_TIMEOUT:?}"
                ))
            })?
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;
        Ok(Self { conn })
    }

    fn key(id: Uuid) -> String {
        format!("{KEY_PREFIX}{id}")
    }
}

#[async_trait]
impl ProjectStore for RedisProjectStore {
    async fn save(&self, project: &Project) -> Result<()> {
        let mut conn = self.conn.clone();
        let json =
            serde_json::to_string(project).map_err(|e| AutoForgeError::Store(e.to_string()))?;
        conn.set::<_, _, ()>(Self::key(project.id.0), json)
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Project>> {
        let mut conn = self.conn.clone();
        let json: Option<String> = conn
            .get(Self::key(id))
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;
        match json {
            Some(s) => {
                let p =
                    serde_json::from_str(&s).map_err(|e| AutoForgeError::Store(e.to_string()))?;
                Ok(Some(p))
            }
            None => Ok(None),
        }
    }

    async fn list(&self) -> Result<Vec<Project>> {
        let mut conn = self.conn.clone();
        let mut projects = Vec::new();
        let mut cursor: u64 = 0;

        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{KEY_PREFIX}*"))
                .arg("COUNT")
                .arg(200)
                .query_async(&mut conn)
                .await
                .map_err(|e| AutoForgeError::Store(e.to_string()))?;

            for key in keys {
                let json: Option<String> = conn
                    .get(&key)
                    .await
                    .map_err(|e| AutoForgeError::Store(e.to_string()))?;
                if let Some(s) = json {
                    match serde_json::from_str(&s) {
                        Ok(p) => projects.push(p),
                        Err(e) => {
                            tracing::warn!(key, error = %e, "skipping corrupt project record")
                        }
                    }
                }
            }

            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(projects)
    }
}
