use crate::clients::omniroute::AiResponse;
use crate::error::{AutoForgeError, Result};
use crate::services::ai::{project_spec_system, ProjectSpec, TokenPolicy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
pub(super) struct ExtractionCache {
    pub fingerprint: String,
    pub spec: ProjectSpec,
    pub model: String,
}

pub(super) fn fingerprint(source: &str, model: &str, policy: &TokenPolicy) -> String {
    let mut digest = Sha256::new();
    for part in ["extract-v2", project_spec_system(), source, model] {
        digest.update(part.len().to_le_bytes());
        digest.update(part.as_bytes());
    }
    digest.update(policy.extract_output_tokens.to_le_bytes());
    hex::encode(digest.finalize())
}

pub(super) fn parse_spec(content: &str) -> Result<ProjectSpec> {
    let value: serde_json::Value = crate::services::ai::parse_json(content)?;
    let shape = serde_json::to_value(ProjectSpec::default())
        .map_err(|error| AutoForgeError::Internal(error.to_string()))?;
    let expected = shape
        .as_object()
        .ok_or_else(|| AutoForgeError::Internal("invalid spec schema".into()))?;
    for key in expected.keys() {
        if value.get(key).is_none() {
            return Err(AutoForgeError::OmniRoute(format!(
                "project_spec missing required field {key}"
            )));
        }
    }
    serde_json::from_value(value)
        .map_err(|error| AutoForgeError::OmniRoute(format!("invalid project_spec: {error}")))
}

pub(super) fn usage_metadata(responses: &[AiResponse]) -> serde_json::Value {
    let cost: Option<f64> = responses
        .iter()
        .map(|response| response.usage.cost_usd)
        .sum();
    serde_json::json!({
        "input_tokens": responses.iter().map(|response| response.usage.input_tokens).sum::<u64>(),
        "output_tokens": responses.iter().map(|response| response.usage.output_tokens).sum::<u64>(),
        "cached_tokens": responses.iter().map(|response| response.usage.cached_tokens).sum::<u64>(),
        "cost_usd": cost,
        "calls": responses.iter().map(|response| serde_json::json!({"model": response.model, "usage": response.usage})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_changes_with_source_model_and_output_policy() {
        let policy = TokenPolicy::default();
        let key = fingerprint("source", "model", &policy);
        assert_eq!(key, fingerprint("source", "model", &policy));
        assert_ne!(key, fingerprint("changed source", "model", &policy));
        assert_ne!(key, fingerprint("source", "different-model", &policy));
        let smaller = TokenPolicy {
            extract_output_tokens: 1000,
            ..policy
        };
        assert_ne!(key, fingerprint("source", "model", &smaller));
    }

    #[test]
    fn missing_fields_and_wrong_field_types_are_rejected() {
        assert!(parse_spec(r#"{"title":"API","project_goal":"serve"}"#).is_err());
        let spec = serde_json::to_string(&ProjectSpec::default()).expect("serialize fixture");
        assert!(parse_spec(&spec).is_ok());
        assert!(
            parse_spec(&spec.replace("\"ui_requirements\":[]", "\"ui_requirements\":null"))
                .is_err()
        );
    }
}
