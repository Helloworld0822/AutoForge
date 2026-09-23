use super::{
    summarize, CallDescriptor, CallStatus, LedgerRecord, Reservation, UsageLedger, UsageSummary,
};
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Script};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const KEY_PREFIX: &str = "autoforge:usage:";

/// Redis를 권위 있는 원장으로 사용한다. 여러 워커/재시작에 걸쳐 예약과 정산이 원자적이다.
pub struct RedisUsageLedger {
    conn: ConnectionManager,
}

impl RedisUsageLedger {
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

    fn records_key(project_id: &str) -> String {
        format!("{KEY_PREFIX}{project_id}:records")
    }

    fn total_key(project_id: &str) -> String {
        format!("{KEY_PREFIX}{project_id}:total")
    }

    fn task_total_key(project_id: &str, task_id: &str) -> String {
        format!("{KEY_PREFIX}{project_id}:task:{task_id}:total")
    }
}

const RESERVE_SCRIPT: &str = r#"
local records = KEYS[1]
local total = KEYS[2]
local task_total = KEYS[3]
local call_id = ARGV[1]
local quote = tonumber(ARGV[2])
local project_limit = tonumber(ARGV[3])
local task_limit = tonumber(ARGV[4])
local record_json = ARGV[5]
if redis.call('HEXISTS', records, call_id) == 1 then
  return {'duplicate', redis.call('HGET', records, call_id)}
end
local current = tonumber(redis.call('GET', total) or '0')
if current + quote > project_limit then
  return {'overbudget', tostring(current), 'project'}
end
if task_total ~= '' then
  local task_current = tonumber(redis.call('GET', task_total) or '0')
  if task_current + quote > task_limit then
    return {'overbudget', tostring(task_current), 'task'}
  end
  redis.call('INCRBY', task_total, quote)
end
redis.call('HSET', records, call_id, record_json)
redis.call('INCRBY', total, quote)
return {'granted'}
"#;

const SETTLE_SCRIPT: &str = r#"
local records = KEYS[1]
local total = KEYS[2]
local task_total = KEYS[3]
local call_id = ARGV[1]
local record_json = ARGV[2]
local delta = tonumber(ARGV[3])
if redis.call('HEXISTS', records, call_id) == 0 then
  return {'missing'}
end
redis.call('HSET', records, call_id, record_json)
if delta ~= 0 then
  redis.call('INCRBY', total, delta)
  if task_total ~= '' then
    redis.call('INCRBY', task_total, delta)
  end
end
return {'ok'}
"#;

#[async_trait]
impl UsageLedger for RedisUsageLedger {
    async fn reserve(
        &self,
        call: &CallDescriptor,
        quote_micro_usd: u64,
        project_limit_micro_usd: u64,
        task_limit_micro_usd: u64,
    ) -> Result<Reservation> {
        let record = LedgerRecord {
            call: call.clone(),
            reserved_micro_usd: quote_micro_usd,
            committed_micro_usd: quote_micro_usd,
            status: CallStatus::Reserved,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        let record_json =
            serde_json::to_string(&record).map_err(|e| AutoForgeError::Internal(e.to_string()))?;

        let task_key = call
            .task_id
            .as_ref()
            .map(|task| Self::task_total_key(&call.project_id, task))
            .unwrap_or_default();

        let mut conn = self.conn.clone();
        let result: Vec<String> = Script::new(RESERVE_SCRIPT)
            .key(Self::records_key(&call.project_id))
            .key(Self::total_key(&call.project_id))
            .key(task_key)
            .arg(&call.call_id)
            .arg(quote_micro_usd)
            .arg(project_limit_micro_usd)
            .arg(task_limit_micro_usd)
            .arg(record_json)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;

        match result.first().map(String::as_str) {
            Some("granted") => Ok(Reservation::Granted),
            Some("duplicate") => {
                let status = result
                    .get(1)
                    .and_then(|json| serde_json::from_str::<LedgerRecord>(json).ok())
                    .map(|record| record.status)
                    .unwrap_or(CallStatus::Reserved);
                Ok(Reservation::Duplicate(status))
            }
            Some("overbudget") => {
                let current = result
                    .get(1)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(0);
                let task = result.get(2).map(String::as_str) == Some("task");
                Ok(Reservation::OverBudget {
                    limit_micro_usd: if task {
                        task_limit_micro_usd
                    } else {
                        project_limit_micro_usd
                    },
                    would_be_micro_usd: current + quote_micro_usd,
                    task,
                })
            }
            other => Err(AutoForgeError::Store(format!(
                "unexpected ledger reserve result: {other:?}"
            ))),
        }
    }

    async fn settle(
        &self,
        call_id: &str,
        actual_micro_usd: Option<u64>,
        status: CallStatus,
    ) -> Result<()> {
        let mut conn = self.conn.clone();
        let mut records = self
            .project_of_call(call_id)
            .await?
            .ok_or_else(|| AutoForgeError::Store(format!("ledger record {call_id} not found")))?;

        let record = records
            .iter_mut()
            .find(|record| record.call.call_id == call_id)
            .ok_or_else(|| AutoForgeError::Store(format!("ledger record {call_id} not found")))?;

        let committed = actual_micro_usd.unwrap_or(record.reserved_micro_usd);
        let delta = committed as i64 - record.reserved_micro_usd as i64;
        record.status = status;
        record.committed_micro_usd = committed;
        let record_json =
            serde_json::to_string(record).map_err(|e| AutoForgeError::Internal(e.to_string()))?;
        let task_key = record
            .call
            .task_id
            .as_ref()
            .map(|task| Self::task_total_key(&record.call.project_id, task))
            .unwrap_or_default();

        let _: Vec<String> = Script::new(SETTLE_SCRIPT)
            .key(Self::records_key(&record.call.project_id))
            .key(Self::total_key(&record.call.project_id))
            .key(task_key)
            .arg(call_id)
            .arg(record_json)
            .arg(delta)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;
        Ok(())
    }

    async fn summary(&self, project_id: &str) -> Result<UsageSummary> {
        let mut conn = self.conn.clone();
        let entries: std::collections::HashMap<String, String> = conn
            .hgetall(Self::records_key(project_id))
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;
        let records = entries
            .into_values()
            .filter_map(|json| serde_json::from_str::<LedgerRecord>(&json).ok());
        Ok(summarize(records))
    }
}

impl RedisUsageLedger {
    async fn project_of_call(&self, call_id: &str) -> Result<Option<Vec<LedgerRecord>>> {
        let mut conn = self.conn.clone();
        let project_id = call_id.split(':').next().unwrap_or_default().to_string();
        let entries: std::collections::HashMap<String, String> = conn
            .hgetall(Self::records_key(&project_id))
            .await
            .map_err(|e| AutoForgeError::Store(e.to_string()))?;
        let records: Vec<LedgerRecord> = entries
            .into_values()
            .filter_map(|json| serde_json::from_str::<LedgerRecord>(&json).ok())
            .collect();
        if records.iter().any(|record| record.call.call_id == call_id) {
            Ok(Some(records))
        } else {
            Ok(None)
        }
    }
}
