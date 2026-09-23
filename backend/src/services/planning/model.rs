use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningBundle {
    pub architecture: PlanningDocument,
    pub spec: PlanningDocument,
    pub tasks: Vec<PlanningTask>,
    pub planning_meta: PlanningMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningDocument {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTask {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub task_type: String,
    pub dependencies: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub risk: String,
    pub estimated_context: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningMeta {
    pub complexity: String,
    pub confidence: f64,
    pub requires_escalation: bool,
    pub reason: Option<String>,
}
