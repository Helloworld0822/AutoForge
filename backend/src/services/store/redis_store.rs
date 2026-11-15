use super::ProjectStore;
use crate::domain::Project;
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use redis::aio::{ConnectionLike, ConnectionManager};
use redis::AsyncCommands;
use std::collections::HashSet;
use uuid::Uuid;

const KEY_PREFIX: &str = "autoforge:project:";
const READ_BATCH_SIZE: usize = 200;

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
        list_projects(&mut self.conn.clone()).await
    }
}

async fn list_projects(conn: &mut impl ConnectionLike) -> Result<Vec<Project>> {
    let mut projects = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut cursor: u64 = 0;

    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(format!("{KEY_PREFIX}*"))
            .arg("COUNT")
            .arg(READ_BATCH_SIZE)
            .query_async(conn)
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;

        // SCAN can repeat keys, including within a single page.
        let keys: Vec<_> = keys
            .into_iter()
            .filter(|key| seen_keys.insert(key.clone()))
            .collect();
        // COUNT is only a hint, so bound each pipeline independently of the page size.
        for batch in keys.chunks(READ_BATCH_SIZE) {
            let mut pipeline = redis::pipe();
            for key in batch {
                pipeline.get(key);
            }
            // Pipelined GET preserves wrong-type errors, unlike MGET.
            let records: Vec<Option<String>> = pipeline
                .query_async(conn)
                .await
                .map_err(|e| AutoForgeError::Store(e.to_string()))?;
            for (key, json) in batch.iter().zip(records) {
                if let Some(json) = json {
                    match serde_json::from_str(&json) {
                        Ok(project) => projects.push(project),
                        Err(e) => {
                            tracing::warn!(key, error = %e, "skipping corrupt project record")
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use redis::{RedisFuture, RedisResult, Value};
    use std::collections::VecDeque;

    #[derive(Default)]
    struct ScriptedConnection {
        steps: VecDeque<(Vec<u8>, RedisResult<Value>)>,
    }

    impl ScriptedConnection {
        fn scan(&mut self, cursor: u64, next: u64, keys: &[String]) {
            let command = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(format!("{KEY_PREFIX}*"))
                .arg("COUNT")
                .arg(READ_BATCH_SIZE)
                .get_packed_command();
            self.steps.push_back((
                command,
                Ok(Value::Array(vec![
                    Value::BulkString(next.to_string().into_bytes()),
                    Value::Array(
                        keys.iter()
                            .map(|key| Value::BulkString(key.as_bytes().to_vec()))
                            .collect(),
                    ),
                ])),
            ));
        }

        fn reads(&mut self, keys: &[String], result: RedisResult<Value>) {
            let mut pipeline = redis::pipe();
            for key in keys {
                pipeline.get(key);
            }
            self.steps
                .push_back((pipeline.get_packed_pipeline(), result));
        }

        fn respond(&mut self, command: Vec<u8>) -> RedisResult<Value> {
            let (expected, response) = self.steps.pop_front().expect("unexpected Redis request");
            assert_eq!(command, expected);
            response
        }
    }

    impl ConnectionLike for ScriptedConnection {
        fn req_packed_command<'a>(&'a mut self, cmd: &'a redis::Cmd) -> RedisFuture<'a, Value> {
            Box::pin(async move { self.respond(cmd.get_packed_command()) })
        }

        fn req_packed_commands<'a>(
            &'a mut self,
            cmd: &'a redis::Pipeline,
            offset: usize,
            count: usize,
        ) -> RedisFuture<'a, Vec<Value>> {
            Box::pin(async move {
                assert_eq!(offset, 0);
                assert_eq!(count, cmd.cmd_iter().count());
                match self.respond(cmd.get_packed_pipeline())? {
                    Value::Array(values) => Ok(values),
                    other => panic!("expected pipeline response, got {other:?}"),
                }
            })
        }

        fn get_db(&self) -> i64 {
            0
        }
    }

    fn project_record(id: Uuid) -> Value {
        let project: Project = serde_json::from_value(serde_json::json!({
            "id": id,
            "name": "Stored project",
            "state": "running",
            "stages": {},
            "scheduler": crate::services::orchestrator::DagScheduler::default(),
            "stage_outputs": {},
            "accumulated_artifacts": []
        }))
        .unwrap();
        Value::BulkString(serde_json::to_vec(&project).unwrap())
    }

    #[tokio::test]
    async fn list_batches_reads_and_skips_duplicate_missing_and_corrupt_records() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let keys = vec![
            RedisProjectStore::key(first),
            RedisProjectStore::key(Uuid::new_v4()),
            RedisProjectStore::key(Uuid::new_v4()),
        ];
        let second_key = RedisProjectStore::key(second);
        let mut conn = ScriptedConnection::default();
        let mut duplicate_page = keys.clone();
        duplicate_page.push(keys[0].clone());
        conn.scan(0, 1, &duplicate_page);
        conn.reads(
            &keys,
            Ok(Value::Array(vec![
                project_record(first),
                Value::Nil,
                Value::BulkString(b"invalid JSON".to_vec()),
            ])),
        );
        // An empty page with a nonzero cursor must not end the scan or issue GETs.
        conn.scan(1, 2, &[]);
        conn.scan(2, 3, &keys);
        conn.scan(3, 0, &[keys[0].clone(), second_key.clone()]);
        conn.reads(
            &[second_key],
            Ok(Value::Array(vec![project_record(second)])),
        );

        let projects = list_projects(&mut conn).await.unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].id.0, first);
        assert_eq!(projects[1].id.0, second);
        assert!(conn.steps.is_empty());
    }

    #[tokio::test]
    async fn list_bounds_pipelines_when_scan_returns_more_than_count() {
        let keys: Vec<_> = (0..READ_BATCH_SIZE + 1)
            .map(|_| RedisProjectStore::key(Uuid::new_v4()))
            .collect();
        let mut conn = ScriptedConnection::default();
        conn.scan(0, 0, &keys);
        conn.reads(
            &keys[..READ_BATCH_SIZE],
            Ok(Value::Array(vec![Value::Nil; READ_BATCH_SIZE])),
        );
        conn.reads(&keys[READ_BATCH_SIZE..], Ok(Value::Array(vec![Value::Nil])));

        assert!(list_projects(&mut conn).await.unwrap().is_empty());
        assert!(conn.steps.is_empty());
    }

    #[tokio::test]
    async fn list_empty_store_does_not_send_an_empty_pipeline() {
        let mut conn = ScriptedConnection::default();
        conn.scan(0, 0, &[]);
        assert!(list_projects(&mut conn).await.unwrap().is_empty());
        assert!(conn.steps.is_empty());
    }

    #[tokio::test]
    async fn list_propagates_redis_read_errors() {
        let keys = vec![RedisProjectStore::key(Uuid::new_v4())];
        let mut conn = ScriptedConnection::default();
        conn.scan(0, 0, &keys);
        conn.reads(
            &keys,
            Err((
                redis::ErrorKind::TypeError,
                "WRONGTYPE unexpected record type",
            )
                .into()),
        );
        let error = list_projects(&mut conn).await.unwrap_err();
        assert!(matches!(error, AutoForgeError::Store(_)));
        assert!(error.to_string().contains("WRONGTYPE"));
        assert!(conn.steps.is_empty());
    }
}
