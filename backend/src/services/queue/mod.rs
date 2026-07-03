pub mod messages;

use self::messages::{PipelineEvent, QueueCommand};
use crate::config::Config;
use crate::domain::StageCommand;
use crate::error::{AutoForgeError, Result};
use redis::aio::ConnectionManager;
use std::sync::Arc;
use tracing::{debug, info};

pub use messages::{PipelineEvent as Event, QueueCommand as Command};

pub struct MessageQueue {
    conn: ConnectionManager,
    pub commands_stream: String,
    pub events_stream: String,
    pub consumer_group: String,
}

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

impl MessageQueue {
    pub async fn connect(config: &Config) -> Result<Arc<Self>> {
        let client = redis::Client::open(config.redis_url.as_str())
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        let conn = tokio::time::timeout(CONNECT_TIMEOUT, client.get_connection_manager())
            .await
            .map_err(|_| {
                AutoForgeError::Queue(format!(
                    "timed out connecting to Redis at {} after {CONNECT_TIMEOUT:?}",
                    config.redis_url
                ))
            })?
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        let mq = Arc::new(Self {
            conn,
            commands_stream: config.queue_commands_stream.clone(),
            events_stream: config.queue_events_stream.clone(),
            consumer_group: config.queue_consumer_group.clone(),
        });

        mq.ensure_groups().await?;
        Ok(mq)
    }

    pub async fn ping(&self) -> Result<()> {
        redis::cmd("PING")
            .query_async::<String>(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(())
    }

    async fn ensure_groups(&self) -> Result<()> {
        for stream in [&self.commands_stream, &self.events_stream] {
            let result: redis::RedisResult<String> = redis::cmd("XGROUP")
                .arg("CREATE")
                .arg(stream)
                .arg(&self.consumer_group)
                .arg("0")
                .arg("MKSTREAM")
                .query_async(&mut self.conn.clone())
                .await;
            match result {
                Ok(_) => info!(stream, group = %self.consumer_group, "consumer group created"),
                Err(e) if e.to_string().contains("BUSYGROUP") => {
                    debug!(stream, "consumer group already exists");
                }
                Err(e) => return Err(AutoForgeError::Queue(e.to_string())),
            }
        }
        Ok(())
    }

    pub async fn enqueue_command(&self, cmd: &StageCommand) -> Result<String> {
        let payload = serde_json::to_string(&QueueCommand::from(cmd.clone()))
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        let id: String = redis::cmd("XADD")
            .arg(&self.commands_stream)
            .arg("*")
            .arg("data")
            .arg(&payload)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        debug!(%id, stage = ?cmd.stage, "command enqueued");
        Ok(id)
    }

    pub async fn enqueue_commands(&self, cmds: &[StageCommand]) -> Result<()> {
        for cmd in cmds {
            self.enqueue_command(cmd).await?;
        }
        Ok(())
    }

    pub async fn publish_event(&self, event: &PipelineEvent) -> Result<String> {
        let payload =
            serde_json::to_string(event).map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        let id: String = redis::cmd("XADD")
            .arg(&self.events_stream)
            .arg("*")
            .arg("data")
            .arg(&payload)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(id)
    }

    pub async fn read_commands(
        &self,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Vec<(String, QueueCommand)>> {
        self.read_stream(&self.commands_stream, consumer, count, block_ms)
            .await
    }

    pub async fn read_events(
        &self,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Vec<(String, PipelineEvent)>> {
        self.read_stream(&self.events_stream, consumer, count, block_ms)
            .await
    }

    async fn read_stream<T: serde::de::DeserializeOwned>(
        &self,
        stream: &str,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Vec<(String, T)>> {
        let result: redis::Value = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(&self.consumer_group)
            .arg(consumer)
            .arg("COUNT")
            .arg(count)
            .arg("BLOCK")
            .arg(block_ms)
            .arg("STREAMS")
            .arg(stream)
            .arg(">")
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        Ok(parse_stream_messages(result))
    }

    pub async fn ack_command(&self, id: &str) -> Result<()> {
        self.ack(&self.commands_stream, id).await
    }

    pub async fn ack_event(&self, id: &str) -> Result<()> {
        self.ack(&self.events_stream, id).await
    }

    async fn ack(&self, stream: &str, id: &str) -> Result<()> {
        let _: i32 = redis::cmd("XACK")
            .arg(stream)
            .arg(&self.consumer_group)
            .arg(id)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(())
    }

    /// 처리되지 않은 채 오래 대기 중인(min_idle_ms 이상) pending 메시지를 재소유하여
    /// 재시도 대상으로 가져온다 (컨슈머 크래시 복구 + 실패 후 재시도).
    pub async fn claim_stale_events(
        &self,
        consumer: &str,
        min_idle_ms: i64,
        count: usize,
    ) -> Result<Vec<(String, PipelineEvent)>> {
        self.claim_stale(&self.events_stream, consumer, min_idle_ms, count)
            .await
    }

    pub async fn claim_stale_commands(
        &self,
        consumer: &str,
        min_idle_ms: i64,
        count: usize,
    ) -> Result<Vec<(String, QueueCommand)>> {
        self.claim_stale(&self.commands_stream, consumer, min_idle_ms, count)
            .await
    }

    async fn claim_stale<T: serde::de::DeserializeOwned>(
        &self,
        stream: &str,
        consumer: &str,
        min_idle_ms: i64,
        count: usize,
    ) -> Result<Vec<(String, T)>> {
        let result: redis::Value = redis::cmd("XAUTOCLAIM")
            .arg(stream)
            .arg(&self.consumer_group)
            .arg(consumer)
            .arg(min_idle_ms)
            .arg("0-0")
            .arg("COUNT")
            .arg(count)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        let redis::Value::Array(parts) = result else {
            return Ok(vec![]);
        };
        if parts.len() < 2 {
            return Ok(vec![]);
        }
        let redis::Value::Array(messages) = &parts[1] else {
            return Ok(vec![]);
        };

        let mut out = Vec::new();
        for msg in messages {
            let redis::Value::Array(msg_parts) = msg else {
                continue;
            };
            if msg_parts.len() < 2 {
                continue;
            }
            let id = match &msg_parts[0] {
                redis::Value::BulkString(d) => String::from_utf8_lossy(d).to_string(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => continue,
            };
            if let Some(data) = extract_data_field(&msg_parts[1]) {
                if let Ok(parsed) = serde_json::from_str::<T>(&data) {
                    out.push((id, parsed));
                }
            }
        }
        Ok(out)
    }

    /// 실패 재시도 횟수 카운터 (스트림+메시지 ID 단위, 24시간 TTL)
    pub async fn incr_retry(&self, stream: &str, id: &str) -> Result<i64> {
        let key = format!("autoforge:retry:{stream}:{id}");
        let mut conn = self.conn.clone();
        let n: i64 = redis::cmd("INCR")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        let _: std::result::Result<(), _> = redis::cmd("EXPIRE")
            .arg(&key)
            .arg(86_400)
            .query_async::<()>(&mut conn)
            .await;
        Ok(n)
    }

    /// 최대 재시도 초과 메시지를 데드레터 스트림(`{stream}:dlq`)으로 이동
    pub async fn dead_letter(
        &self,
        stream: &str,
        id: &str,
        payload: &str,
        error: &str,
    ) -> Result<()> {
        let dlq_stream = format!("{stream}:dlq");
        let _: String = redis::cmd("XADD")
            .arg(&dlq_stream)
            .arg("*")
            .arg("original_id")
            .arg(id)
            .arg("error")
            .arg(error)
            .arg("data")
            .arg(payload)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(())
    }
}

fn parse_stream_messages<T: serde::de::DeserializeOwned>(value: redis::Value) -> Vec<(String, T)> {
    let mut out = Vec::new();
    let redis::Value::Array(streams) = value else {
        return out;
    };
    for stream_entry in streams {
        let redis::Value::Array(parts) = stream_entry else {
            continue;
        };
        if parts.len() < 2 {
            continue;
        }
        let redis::Value::Array(messages) = &parts[1] else {
            continue;
        };
        for msg in messages {
            let redis::Value::Array(msg_parts) = msg else {
                continue;
            };
            if msg_parts.len() < 2 {
                continue;
            }
            let id = match &msg_parts[0] {
                redis::Value::BulkString(d) => String::from_utf8_lossy(d).to_string(),
                redis::Value::SimpleString(s) => s.clone(),
                _ => continue,
            };
            let data = extract_data_field(&msg_parts[1]);
            if let Some(data) = data {
                if let Ok(parsed) = serde_json::from_str::<T>(&data) {
                    out.push((id, parsed));
                }
            }
        }
    }
    out
}

fn extract_data_field(value: &redis::Value) -> Option<String> {
    let redis::Value::Array(fields) = value else {
        return None;
    };
    let mut iter = fields.iter();
    while let Some(key) = iter.next() {
        let val = iter.next()?;
        let key_str = match key {
            redis::Value::BulkString(d) => String::from_utf8_lossy(d),
            redis::Value::SimpleString(s) => s.as_str().into(),
            _ => continue,
        };
        if key_str == "data" {
            return match val {
                redis::Value::BulkString(d) => Some(String::from_utf8_lossy(d).to_string()),
                redis::Value::SimpleString(s) => Some(s.clone()),
                _ => None,
            };
        }
    }
    None
}
