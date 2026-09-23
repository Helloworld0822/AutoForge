use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

pub const MICRO_PER_USD: u64 = 1_000_000;

pub fn usd_to_micro(usd: f64) -> u64 {
    if !usd.is_finite() || usd <= 0.0 {
        return 0;
    }
    (usd * MICRO_PER_USD as f64).round() as u64
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallDescriptor {
    pub call_id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    pub stage: String,
    pub model: String,
    pub purpose: String,
}

impl CallDescriptor {
    /// (project, stage, attempt, slot)에서 재전달에도 동일하게 유도되는 안정적인 호출 ID.
    pub fn stable_id(project_id: &str, stage: &str, attempt: u8, slot: &str) -> String {
        format!("{project_id}:{stage}:{attempt}:{slot}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Reserved,
    Settled,
    FailedKnownFree,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub call: CallDescriptor,
    pub reserved_micro_usd: u64,
    pub committed_micro_usd: u64,
    pub status: CallStatus,
    pub timestamp: String,
}

impl LedgerRecord {
    fn committed(&self) -> u64 {
        self.committed_micro_usd
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageSummary {
    pub committed_micro_usd: u64,
    pub reserved_micro_usd: u64,
    pub unknown_calls: u64,
    pub records: Vec<LedgerRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    Granted,
    Duplicate(CallStatus),
    OverBudget {
        limit_micro_usd: u64,
        would_be_micro_usd: u64,
        task: bool,
    },
}

#[async_trait]
pub trait UsageLedger: Send + Sync {
    async fn reserve(
        &self,
        call: &CallDescriptor,
        quote_micro_usd: u64,
        project_limit_micro_usd: u64,
        task_limit_micro_usd: u64,
    ) -> Result<Reservation>;

    async fn settle(
        &self,
        call_id: &str,
        actual_micro_usd: Option<u64>,
        status: CallStatus,
    ) -> Result<()>;

    async fn summary(&self, project_id: &str) -> Result<UsageSummary>;
}

fn summarize(records: impl Iterator<Item = LedgerRecord>) -> UsageSummary {
    let mut summary = UsageSummary::default();
    for record in records {
        if record.status == CallStatus::OutcomeUnknown {
            summary.unknown_calls += 1;
        }
        if record.status == CallStatus::Reserved {
            summary.reserved_micro_usd += record.reserved_micro_usd;
        }
        summary.committed_micro_usd += record.committed();
        summary.records.push(record);
    }
    summary
}

#[derive(Default)]
pub struct MemoryUsageLedger {
    projects: Mutex<HashMap<String, HashMap<String, LedgerRecord>>>,
}

impl MemoryUsageLedger {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl UsageLedger for MemoryUsageLedger {
    async fn reserve(
        &self,
        call: &CallDescriptor,
        quote_micro_usd: u64,
        project_limit_micro_usd: u64,
        task_limit_micro_usd: u64,
    ) -> Result<Reservation> {
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| AutoForgeError::Internal("usage ledger lock poisoned".into()))?;
        let records = projects.entry(call.project_id.clone()).or_default();

        if let Some(existing) = records.get(&call.call_id) {
            return Ok(Reservation::Duplicate(existing.status));
        }

        let project_total: u64 = records.values().map(LedgerRecord::committed).sum();
        if project_total + quote_micro_usd > project_limit_micro_usd {
            return Ok(Reservation::OverBudget {
                limit_micro_usd: project_limit_micro_usd,
                would_be_micro_usd: project_total + quote_micro_usd,
                task: false,
            });
        }
        if let Some(task_id) = &call.task_id {
            let task_total: u64 = records
                .values()
                .filter(|record| record.call.task_id.as_deref() == Some(task_id.as_str()))
                .map(LedgerRecord::committed)
                .sum();
            if task_total + quote_micro_usd > task_limit_micro_usd {
                return Ok(Reservation::OverBudget {
                    limit_micro_usd: task_limit_micro_usd,
                    would_be_micro_usd: task_total + quote_micro_usd,
                    task: true,
                });
            }
        }

        records.insert(
            call.call_id.clone(),
            LedgerRecord {
                call: call.clone(),
                reserved_micro_usd: quote_micro_usd,
                committed_micro_usd: quote_micro_usd,
                status: CallStatus::Reserved,
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        );
        Ok(Reservation::Granted)
    }

    async fn settle(
        &self,
        call_id: &str,
        actual_micro_usd: Option<u64>,
        status: CallStatus,
    ) -> Result<()> {
        let mut projects = self
            .projects
            .lock()
            .map_err(|_| AutoForgeError::Internal("usage ledger lock poisoned".into()))?;
        for records in projects.values_mut() {
            if let Some(record) = records.get_mut(call_id) {
                record.status = status;
                record.committed_micro_usd = actual_micro_usd.unwrap_or(record.reserved_micro_usd);
                return Ok(());
            }
        }
        Ok(())
    }

    async fn summary(&self, project_id: &str) -> Result<UsageSummary> {
        let projects = self
            .projects
            .lock()
            .map_err(|_| AutoForgeError::Internal("usage ledger lock poisoned".into()))?;
        let records = projects
            .get(project_id)
            .map(|records| records.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(summarize(records.into_iter()))
    }
}

pub mod redis_ledger;

pub use redis_ledger::RedisUsageLedger;

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, task: Option<&str>) -> CallDescriptor {
        CallDescriptor {
            call_id: id.into(),
            project_id: "p".into(),
            task_id: task.map(str::to_string),
            stage: "extract".into(),
            model: "m".into(),
            purpose: "extraction".into(),
        }
    }

    #[tokio::test]
    async fn reserves_within_budget_and_denies_over_budget() {
        let ledger = MemoryUsageLedger::new();
        assert_eq!(
            ledger
                .reserve(&call("a", None), 900_000, 1_000_000, 1_000_000)
                .await
                .unwrap(),
            Reservation::Granted
        );
        match ledger
            .reserve(&call("b", None), 200_000, 1_000_000, 1_000_000)
            .await
            .unwrap()
        {
            Reservation::OverBudget { task, .. } => assert!(!task),
            other => panic!("expected over budget, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exact_limit_is_allowed() {
        let ledger = MemoryUsageLedger::new();
        assert_eq!(
            ledger
                .reserve(&call("a", None), 1_000_000, 1_000_000, 1_000_000)
                .await
                .unwrap(),
            Reservation::Granted
        );
        assert!(matches!(
            ledger
                .reserve(&call("b", None), 1, 1_000_000, 1_000_000)
                .await
                .unwrap(),
            Reservation::OverBudget { .. }
        ));
    }

    #[tokio::test]
    async fn duplicate_call_ids_are_not_reserved_twice() {
        let ledger = MemoryUsageLedger::new();
        ledger
            .reserve(&call("same", None), 100_000, 1_000_000, 1_000_000)
            .await
            .unwrap();
        assert_eq!(
            ledger
                .reserve(&call("same", None), 100_000, 1_000_000, 1_000_000)
                .await
                .unwrap(),
            Reservation::Duplicate(CallStatus::Reserved)
        );
        let summary = ledger.summary("p").await.unwrap();
        assert_eq!(summary.records.len(), 1);
        assert_eq!(summary.committed_micro_usd, 100_000);
    }

    #[tokio::test]
    async fn settle_with_actual_cost_releases_the_over_reservation() {
        let ledger = MemoryUsageLedger::new();
        ledger
            .reserve(&call("a", None), 500_000, 1_000_000, 1_000_000)
            .await
            .unwrap();
        ledger
            .settle("a", Some(120_000), CallStatus::Settled)
            .await
            .unwrap();
        let summary = ledger.summary("p").await.unwrap();
        assert_eq!(summary.committed_micro_usd, 120_000);
        assert_eq!(summary.unknown_calls, 0);
    }

    #[tokio::test]
    async fn unknown_outcome_keeps_the_reservation() {
        let ledger = MemoryUsageLedger::new();
        ledger
            .reserve(&call("a", None), 500_000, 1_000_000, 1_000_000)
            .await
            .unwrap();
        ledger
            .settle("a", None, CallStatus::OutcomeUnknown)
            .await
            .unwrap();
        let summary = ledger.summary("p").await.unwrap();
        assert_eq!(summary.committed_micro_usd, 500_000);
        assert_eq!(summary.unknown_calls, 1);
    }

    #[tokio::test]
    async fn task_budget_is_enforced_independently() {
        let ledger = MemoryUsageLedger::new();
        ledger
            .reserve(&call("a", Some("t1")), 300_000, 10_000_000, 400_000)
            .await
            .unwrap();
        match ledger
            .reserve(&call("b", Some("t1")), 200_000, 10_000_000, 400_000)
            .await
            .unwrap()
        {
            Reservation::OverBudget { task, .. } => assert!(task),
            other => panic!("expected task over budget, got {other:?}"),
        }
        assert_eq!(
            ledger
                .reserve(&call("c", Some("t2")), 200_000, 10_000_000, 400_000)
                .await
                .unwrap(),
            Reservation::Granted
        );
    }

    #[tokio::test]
    async fn concurrent_reservations_cannot_jointly_exceed_the_budget() {
        let ledger = std::sync::Arc::new(MemoryUsageLedger::new());
        let limit = 1_000_000u64;
        let mut handles = Vec::new();
        for index in 0..8 {
            let ledger = ledger.clone();
            handles.push(tokio::spawn(async move {
                ledger
                    .reserve(&call(&format!("c{index}"), None), 300_000, limit, limit)
                    .await
                    .unwrap()
            }));
        }
        let mut granted = 0;
        for handle in handles {
            if matches!(handle.await.unwrap(), Reservation::Granted) {
                granted += 1;
            }
        }
        assert_eq!(
            granted, 3,
            "only three 300k reservations fit in a 1M budget"
        );
        assert!(ledger.summary("p").await.unwrap().committed_micro_usd <= limit);
    }
}
