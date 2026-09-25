use super::ProjectStore;
use crate::domain::Project;
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use uuid::Uuid;

const KEY_PREFIX: &str = "autoforge:project:";
const FETCH_BATCH_SIZE: usize = 200;

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
                .arg(FETCH_BATCH_SIZE)
                .query_async(&mut conn)
                .await
                .map_err(|e| AutoForgeError::Store(e.to_string()))?;

            // SCAN's COUNT is only a hint, so bound each pipeline independently.
            // Keep GET semantics: MGET would silently ignore wrong-type records.
            for batch in keys.chunks(FETCH_BATCH_SIZE) {
                let mut pipeline = redis::pipe();
                for key in batch {
                    pipeline.cmd("GET").arg(key);
                }
                let records: Vec<Option<String>> = pipeline
                    .query_async(&mut conn)
                    .await
                    .map_err(|e| AutoForgeError::Store(e.to_string()))?;

                for (key, json) in batch.iter().zip(records) {
                    if let Some(s) = json {
                        match serde_json::from_str(&s) {
                            Ok(p) => projects.push(p),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{PipelineState, ProjectId};
    use crate::services::orchestrator::DagScheduler;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};

    struct Exchange {
        commands: Vec<Vec<String>>,
        reply: String,
    }

    fn bulk(value: &str) -> String {
        format!("${}\r\n{value}\r\n", value.len())
    }

    fn scan(cursor: u64, next: u64, keys: &[String]) -> Exchange {
        Exchange {
            commands: vec![vec![
                "SCAN".into(),
                cursor.to_string(),
                "MATCH".into(),
                format!("{KEY_PREFIX}*"),
                "COUNT".into(),
                FETCH_BATCH_SIZE.to_string(),
            ]],
            reply: format!(
                "*2\r\n{}*{}\r\n{}",
                bulk(&next.to_string()),
                keys.len(),
                keys.iter().map(|key| bulk(key)).collect::<String>()
            ),
        }
    }

    fn gets(keys: &[String], reply: String) -> Exchange {
        Exchange {
            commands: keys
                .iter()
                .map(|key| vec!["GET".into(), key.clone()])
                .collect(),
            reply,
        }
    }

    async fn read_command(stream: &mut BufReader<TcpStream>) -> Vec<String> {
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert!(line.starts_with('*'), "expected RESP array, got {line:?}");
        let count: usize = line[1..].trim().parse().unwrap();
        let mut command = Vec::with_capacity(count);
        for _ in 0..count {
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            assert!(line.starts_with('$'));
            let len: usize = line[1..].trim().parse().unwrap();
            let mut bytes = vec![0; len + 2];
            stream.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes[len..], b"\r\n");
            bytes.truncate(len);
            command.push(String::from_utf8(bytes).unwrap());
        }
        command
    }

    async fn list_with_script(exchanges: Vec<Exchange>) -> Result<Vec<Project>> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut stream = BufReader::new(stream);
                for exchange in exchanges {
                    // Withhold responses until the entire batch arrives, so
                    // sequential GETs deadlock and fail the outer timeout.
                    for expected in exchange.commands {
                        let mut actual = read_command(&mut stream).await;
                        while actual.first().map(String::as_str) == Some("CLIENT") {
                            stream.get_mut().write_all(b"+OK\r\n").await.unwrap();
                            actual = read_command(&mut stream).await;
                        }
                        assert_eq!(actual, expected);
                    }
                    stream
                        .get_mut()
                        .write_all(exchange.reply.as_bytes())
                        .await
                        .unwrap();
                }
            });
            let store = RedisProjectStore::connect(&format!("redis://{address}/"))
                .await
                .unwrap();
            let result = store.list().await;
            server.await.unwrap();
            result
        })
        .await
        .expect("Redis listing did not complete the scripted exchanges")
    }

    fn project() -> Project {
        let id = ProjectId::new();
        Project {
            id: id.clone(),
            name: Some("Stored project".into()),
            repo_url: None,
            state: PipelineState::Running,
            stages: Default::default(),
            scheduler: DagScheduler::with_quality(id, 3),
            pdf_bytes: None,
            devops_plan: None,
            programming_language: None,
            language_mode: Default::default(),
            resolved_language: None,
            architecture_clarifications: Vec::new(),
            stage_outputs: Default::default(),
            accumulated_artifacts: Vec::new(),
            slack_message_ts: None,
            created_at: chrono::Utc::now(),
            daily_logs: Default::default(),
            model_config: Default::default(),
        }
    }

    #[tokio::test]
    async fn list_paginates_and_skips_missing_or_corrupt_records() {
        let project = project();
        let key = RedisProjectStore::key(project.id.0);
        let keys = vec![
            key.clone(),
            format!("{KEY_PREFIX}missing"),
            format!("{KEY_PREFIX}corrupt"),
        ];
        let json = serde_json::to_string(&project).unwrap();
        let projects = list_with_script(vec![
            scan(0, 7, &[]),
            scan(7, 9, &keys),
            gets(
                &keys,
                format!("{}$-1\r\n{}", bulk(&json), bulk("invalid json")),
            ),
            // SCAN may repeat keys; preserve the existing list behavior.
            scan(9, 0, std::slice::from_ref(&key)),
            gets(&[key], bulk(&json)),
        ])
        .await
        .unwrap();
        assert_eq!(projects.len(), 2);
        for actual in projects {
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(&project).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn list_handles_scan_exceeding_count_hint() {
        let keys: Vec<_> = (0..FETCH_BATCH_SIZE + 1)
            .map(|index| format!("{KEY_PREFIX}{index}"))
            .collect();
        let projects = list_with_script(vec![
            scan(0, 0, &keys),
            gets(
                &keys[..FETCH_BATCH_SIZE],
                "$-1\r\n".repeat(FETCH_BATCH_SIZE),
            ),
            gets(&keys[FETCH_BATCH_SIZE..], "$-1\r\n".into()),
        ])
        .await
        .unwrap();
        assert!(projects.is_empty());
    }

    #[tokio::test]
    async fn list_empty_scan_needs_no_get() {
        assert!(list_with_script(vec![scan(0, 0, &[])])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn list_propagates_scan_and_get_errors() {
        let mut failed_scan = scan(0, 0, &[]);
        failed_scan.reply = "-ERR scan failed\r\n".into();
        let error = list_with_script(vec![failed_scan]).await.unwrap_err();
        assert!(matches!(error, AutoForgeError::Store(message) if message.contains("scan failed")));
        let keys = vec![
            format!("{KEY_PREFIX}wrong-type"),
            format!("{KEY_PREFIX}missing"),
        ];
        let error = list_with_script(vec![
            scan(0, 0, &keys),
            gets(
                &keys,
                "-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$-1\r\n"
                    .into(),
            ),
        ])
        .await
        .unwrap_err();
        assert!(matches!(error, AutoForgeError::Store(message) if message.contains("WRONGTYPE")));
    }
}
