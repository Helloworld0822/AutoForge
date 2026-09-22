use crate::clients::openrouter::TokenUsage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub project_id: String,
    pub task_id: Option<String>,
    pub stage: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd: f64,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageSummary {
    pub total_cost_usd: f64,
    pub models: HashMap<String, f64>,
    pub records: Vec<UsageRecord>,
}

#[derive(Debug, Clone)]
pub struct CostManager {
    pub project_budget_usd: f64,
    pub task_budget_usd: f64,
}

impl CostManager {
    pub fn can_spend(&self, summary: &UsageSummary, task_id: Option<&str>, estimated: f64) -> bool {
        if summary.total_cost_usd + estimated > self.project_budget_usd {
            return false;
        }
        let task_total: f64 = summary
            .records
            .iter()
            .filter(|record| record.task_id.as_deref() == task_id)
            .map(|record| record.cost_usd)
            .sum();
        task_total + estimated <= self.task_budget_usd
    }

    pub fn record(
        summary: &mut UsageSummary,
        project_id: &str,
        task_id: Option<&str>,
        stage: &str,
        model: &str,
        usage: &TokenUsage,
    ) {
        let cost = usage.cost_usd.unwrap_or_default();
        summary.total_cost_usd += cost;
        *summary.models.entry(model.to_string()).or_default() += cost;
        summary.records.push(UsageRecord {
            project_id: project_id.to_string(),
            task_id: task_id.map(str::to_string),
            stage: stage.to_string(),
            model: model.to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            cost_usd: cost,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_project_budget_before_escalation() {
        let manager = CostManager {
            project_budget_usd: 1.0,
            task_budget_usd: 1.0,
        };
        let mut summary = UsageSummary::default();
        CostManager::record(
            &mut summary,
            "project",
            Some("task"),
            "plan",
            "sonnet",
            &TokenUsage {
                cost_usd: Some(0.9),
                ..Default::default()
            },
        );
        assert!(!manager.can_spend(&summary, Some("task"), 0.2));
    }
}
