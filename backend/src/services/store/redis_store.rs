use super::ProjectStore;
use crate::domain::Project;
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use uuid::Uuid;

const KEY_PREFIX: &str = "autoforge:project:";
const KEY_INDEX: &str = "autoforge:index:projects";
const BATCH_SIZE: usize = 128;

pub struct RedisProjectStore {
    conn: ConnectionManager,
}

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn store_error(e: redis::RedisError) -> AutoForgeError {
    AutoForgeError::Store(e.to_string())
}

impl RedisProjectStore {
    pub async fn connect(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url).map_err(store_error)?;
        let mut conn = tokio::time::timeout(CONNECT_TIMEOUT, client.get_connection_manager())
            .await
            .map_err(|_| AutoForgeError::Store("timed out connecting to Redis".into()))?
            .map_err(store_error)?;
        // Reconcile records written by earlier versions. Concurrent backfills and saves
        // only add members, so an interrupted startup can safely restart at cursor zero.
        let mut cursor: u64 = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("autoforge:project:*")
                .arg("COUNT")
                .arg(BATCH_SIZE)
                .query_async(&mut conn)
                .await
                .map_err(store_error)?;
            if !keys.is_empty() {
                conn.sadd::<_, _, ()>(KEY_INDEX, &keys)
                    .await
                    .map_err(store_error)?;
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
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
        let key = Self::key(project.id.0);
        redis::pipe()
            .atomic()
            .set(&key, json)
            .ignore()
            .sadd(KEY_INDEX, &key)
            .ignore()
            .query_async(&mut conn)
            .await
            .map_err(store_error)
    }

    async fn get(&self, id: Uuid) -> Result<Option<Project>> {
        let mut conn = self.conn.clone();
        let json: Option<String> = conn.get(Self::key(id)).await.map_err(store_error)?;
        json.map(|s| serde_json::from_str(&s).map_err(|e| AutoForgeError::Store(e.to_string())))
            .transpose()
    }

    async fn list(&self) -> Result<Vec<Project>> {
        let mut conn = self.conn.clone();
        let keys: Vec<String> = conn.smembers(KEY_INDEX).await.map_err(store_error)?;
        let mut projects = Vec::with_capacity(keys.len());
        for batch in keys.chunks(BATCH_SIZE) {
            let records: Vec<Option<Vec<u8>>> = conn.mget(batch).await.map_err(store_error)?;
            for (key, record) in batch.iter().zip(records) {
                if let Some(data) = record {
                    match serde_json::from_slice(&data) {
                        Ok(p) => projects.push(p),
                        Err(e) => {
                            tracing::warn!(key, error = %e, "skipping corrupt project record")
                        }
                    }
                }
            }
        }
        Ok(projects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Config};

    #[tokio::test]
    #[ignore = "requires an empty disposable Redis at AUTOFORGE_TEST_REDIS_URL"]
    async fn lists_legacy_and_updated_projects_in_batches() {
        let url = std::env::var("AUTOFORGE_TEST_REDIS_URL").unwrap();
        let mut conn = redis::Client::open(url.as_str())
            .unwrap()
            .get_connection_manager()
            .await
            .unwrap();
        let size: usize = redis::cmd("DBSIZE").query_async(&mut conn).await.unwrap();
        assert_eq!(size, 0, "this test needs an empty disposable Redis");
        let mut config = Config::from_env();
        config.artifacts_dir = std::env::temp_dir()
            .join(Uuid::new_v4().to_string())
            .display()
            .to_string();
        let app = App::new(config).await.unwrap();
        let mut project = app
            .create_project(None, None, None, Default::default(), Default::default())
            .await;
        for _ in 0..BATCH_SIZE {
            project.id.0 = Uuid::new_v4();
            conn.set::<_, _, ()>(
                RedisProjectStore::key(project.id.0),
                serde_json::to_vec(&project).unwrap(),
            )
            .await
            .unwrap();
        }
        let store = RedisProjectStore::connect(&url).await.unwrap();
        project.id.0 = Uuid::new_v4();
        store.save(&project).await.unwrap();
        project.name = Some("updated".into());
        store.save(&project).await.unwrap();

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), BATCH_SIZE + 1);
        assert!(listed
            .iter()
            .any(|p| p.id.0 == project.id.0 && p.name == project.name));
        let stats: String = redis::cmd("INFO")
            .arg("commandstats")
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(stats.contains("cmdstat_mget:calls=2,"), "{stats}");
        assert!(stats.contains("cmdstat_smembers:calls=1,"), "{stats}");
        tokio::fs::remove_dir_all(app.media.base_dir())
            .await
            .unwrap();
    }
}
