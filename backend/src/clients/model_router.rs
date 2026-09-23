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
            extract: env_or("OMNIROUTE_MODEL_EXTRACT", ""),
            plan: env_or("OMNIROUTE_MODEL_PLAN", ""),
            plan_escalation: env_or("OMNIROUTE_MODEL_PLAN_ESCALATION", ""),
            code: env_or("OMNIROUTE_MODEL_CODE", ""),
            debug: env_or("OMNIROUTE_MODEL_DEBUG", ""),
            debug_alternative: env_or("OMNIROUTE_MODEL_DEBUG_ALT", ""),
            debug_escalation: env_or("OMNIROUTE_MODEL_DEBUG_ESCALATION", ""),
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

    pub fn debug_role(&self, attempt: u8, deepseek_retries: u8, mid_retries: u8) -> ModelRole {
        if attempt < deepseek_retries {
            ModelRole::Code
        } else if attempt < deepseek_retries.saturating_add(mid_retries) {
            ModelRole::Debug
        } else {
            ModelRole::DebugEscalation
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
        assert_eq!(router.debug_role(0, 2, 1), ModelRole::Code);
        assert_eq!(router.debug_role(2, 2, 1), ModelRole::Debug);
        assert_eq!(router.debug_role(3, 2, 1), ModelRole::DebugEscalation);
    }
}
