use crate::clients::omniroute::{
    AiMessage, AiProvider, AiRequest, AiResponse, AiRole, ResponseFormat,
};
use crate::error::{AutoForgeError, Result};
use serde::{Deserialize, Serialize};
mod policy;
pub use policy::{estimate_tokens, AiPurpose, TokenPolicy};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectSpec {
    pub title: String,
    pub project_goal: String,
    #[serde(default)]
    pub functional_requirements: Vec<String>,
    #[serde(default)]
    pub non_functional_requirements: Vec<String>,
    #[serde(default)]
    pub ui_requirements: Vec<String>,
    #[serde(default)]
    pub technical_constraints: Vec<String>,
    #[serde(default)]
    pub business_constraints: Vec<String>,
    #[serde(default)]
    pub preferred_stack: Vec<String>,
    #[serde(default)]
    pub integrations: Vec<String>,
    #[serde(default)]
    pub security_requirements: Vec<String>,
    #[serde(default)]
    pub deployment_requirements: Vec<String>,
    #[serde(default)]
    pub timeline: Option<String>,
    #[serde(default)]
    pub budget_hint: Option<String>,
    #[serde(default)]
    pub unknowns: Vec<String>,
    #[serde(default)]
    pub contradictions: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionList {
    #[serde(default)]
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    #[serde(alias = "text")]
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningMeta {
    pub complexity: String,
    pub confidence: f64,
    pub requires_escalation: bool,
    pub reason: Option<String>,
}

fn default_true() -> bool {
    true
}

pub async fn complete_json(
    client: &dyn AiProvider,
    model: &str,
    system: &str,
    user: String,
    purpose: AiPurpose,
    policy: &TokenPolicy,
) -> Result<AiResponse> {
    if model.trim().is_empty() {
        return Err(AutoForgeError::OmniRoute("model is not configured; choose an ID from this OmniRoute instance's /v1/models and set OMNIROUTE_MODEL_* or the project override".into()));
    }
    policy.check_input(system, &user)?;
    client
        .complete(AiRequest {
            model: model.to_string(),
            messages: vec![
                AiMessage {
                    role: AiRole::System,
                    content: system.to_string(),
                },
                AiMessage {
                    role: AiRole::User,
                    content: user,
                },
            ],
            temperature: Some(0.1),
            max_tokens: Some(policy.output_tokens(purpose)),
            response_format: Some(ResponseFormat {
                kind: "json_object".into(),
            }),
        })
        .await
}

pub fn parse_json<T: for<'de> Deserialize<'de>>(content: &str) -> Result<T> {
    let trimmed = content.trim();
    let candidate = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    serde_json::from_str(candidate)
        .map_err(|error| AutoForgeError::OmniRoute(format!("invalid structured output: {error}")))
}

pub fn project_spec_system() -> &'static str {
    r#"Extract requirements without summarizing or inventing facts. Return only JSON with ALL these keys: title (string), project_goal (string), functional_requirements, non_functional_requirements, ui_requirements, technical_constraints, business_constraints, preferred_stack, integrations, security_requirements, deployment_requirements, unknowns, contradictions, source_refs (all arrays of strings), timeline and budget_hint (string or null). Preserve numbers, deadlines and page/section references. Use empty arrays/null for information absent from the source. Never infer additional requirements. UI requirements must be empty for an API/library without a UI."#
}
