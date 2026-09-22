use crate::domain::StageId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelRole {
    Extract,
    Plan,
    PlanEscalation,
    Code,
    Debug,
    DebugAlternative,
    DebugEscalation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouter {
    pub extract: String,
    pub plan: String,
    pub plan_escalation: String,
    pub code: String,
    pub debug: String,
    pub debug_alternative: String,
    pub debug_escalation: String,
}

impl ModelRouter {
    pub fn from_env() -> Self {
        Self {
            extract: env_or("OPENROUTER_MODEL_EXTRACT", "openai/gpt-5.6-luna"),
            plan: env_or("OPENROUTER_MODEL_PLAN", "anthropic/claude-sonnet-5"),
            plan_escalation: env_or("OPENROUTER_MODEL_PLAN_ESCALATION", "openai/gpt-6-astra"),
            code: env_or("OPENROUTER_MODEL_CODE", "deepseek/deepseek-v4.1-flash"),
            debug: env_or("OPENROUTER_MODEL_DEBUG", "anthropic/claude-sonnet-5"),
            debug_alternative: env_or("OPENROUTER_MODEL_DEBUG_ALT", "moonshotai/kimi-k3"),
            debug_escalation: env_or(
                "OPENROUTER_MODEL_DEBUG_ESCALATION",
                "anthropic/claude-opus-5",
            ),
        }
    }

    pub fn model(&self, role: ModelRole) -> &str {
        match role {
            ModelRole::Extract => &self.extract,
            ModelRole::Plan => &self.plan,
            ModelRole::PlanEscalation => &self.plan_escalation,
            ModelRole::Code => &self.code,
            ModelRole::Debug => &self.debug,
            ModelRole::DebugAlternative => &self.debug_alternative,
            ModelRole::DebugEscalation => &self.debug_escalation,
        }
    }

    pub fn role_for_stage(&self, stage: StageId) -> Option<ModelRole> {
        match stage {
            StageId::Summarize => Some(ModelRole::Extract),
            StageId::Architect => Some(ModelRole::Plan),
            StageId::Implement => Some(ModelRole::Code),
            StageId::Verify | StageId::SecurityPatch => Some(ModelRole::Debug),
            StageId::Debug => Some(ModelRole::Debug),
            StageId::Ingest | StageId::Design | StageId::Deliver => None,
        }
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_pipeline_roles_to_configured_models() {
        let router = ModelRouter {
            extract: "extract".into(),
            plan: "plan".into(),
            plan_escalation: "astra".into(),
            code: "code".into(),
            debug: "debug".into(),
            debug_alternative: "kimi".into(),
            debug_escalation: "opus".into(),
        };
        assert_eq!(router.model(ModelRole::Extract), "extract");
        assert_eq!(
            router.role_for_stage(StageId::Implement),
            Some(ModelRole::Code)
        );
        assert_eq!(router.role_for_stage(StageId::Design), None);
    }
}
