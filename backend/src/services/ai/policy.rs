use crate::error::{AutoForgeError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPurpose {
    Extract,
    Clarify,
    Plan,
    Diagnosis,
}

#[derive(Debug, Clone)]
pub struct TokenPolicy {
    pub max_input_tokens: usize,
    pub extract_output_tokens: u32,
    pub clarify_output_tokens: u32,
    pub plan_output_tokens: u32,
    pub diagnosis_output_tokens: u32,
}

impl Default for TokenPolicy {
    fn default() -> Self {
        Self {
            max_input_tokens: 30_000,
            extract_output_tokens: 16_000,
            clarify_output_tokens: 2_000,
            plan_output_tokens: 16_000,
            diagnosis_output_tokens: 4_000,
        }
    }
}

impl TokenPolicy {
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_input_tokens: positive_env("AI_MAX_INPUT_TOKENS", defaults.max_input_tokens),
            extract_output_tokens: positive_env(
                "AI_EXTRACT_OUTPUT_TOKENS",
                defaults.extract_output_tokens,
            ),
            clarify_output_tokens: positive_env(
                "AI_CLARIFY_OUTPUT_TOKENS",
                defaults.clarify_output_tokens,
            ),
            plan_output_tokens: positive_env("AI_PLAN_OUTPUT_TOKENS", defaults.plan_output_tokens),
            diagnosis_output_tokens: positive_env(
                "AI_DIAGNOSIS_OUTPUT_TOKENS",
                defaults.diagnosis_output_tokens,
            ),
        }
    }

    pub fn output_tokens(&self, purpose: AiPurpose) -> u32 {
        match purpose {
            AiPurpose::Extract => self.extract_output_tokens,
            AiPurpose::Clarify => self.clarify_output_tokens,
            AiPurpose::Plan => self.plan_output_tokens,
            AiPurpose::Diagnosis => self.diagnosis_output_tokens,
        }
    }

    pub fn check_input(&self, system: &str, user: &str) -> Result<usize> {
        let estimate = estimate_tokens(system)
            .saturating_add(estimate_tokens(user))
            .saturating_add(32);
        if estimate > self.max_input_tokens {
            return Err(AutoForgeError::OmniRoute(format!(
                "estimated input {estimate} exceeds AI_MAX_INPUT_TOKENS={}; source was not truncated; split the document or raise the limit",
                self.max_input_tokens
            )));
        }
        Ok(estimate)
    }
}

// A tokenizer-independent estimate, not a provider billing count. Non-ASCII text
// is charged by UTF-8 byte here so Korean/CJK is not estimated as English words.
pub fn estimate_tokens(text: &str) -> usize {
    let ascii = text.bytes().filter(u8::is_ascii).count();
    ascii.div_ceil(3).saturating_add(text.len() - ascii)
}

fn positive_env<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr + PartialOrd + Default,
{
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| value > &T::default())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorter_purposes_have_smaller_output_caps() {
        let policy = TokenPolicy::default();
        assert!(policy.output_tokens(AiPurpose::Clarify) < policy.output_tokens(AiPurpose::Plan));
        assert!(
            policy.output_tokens(AiPurpose::Diagnosis) < policy.output_tokens(AiPurpose::Extract)
        );
    }

    #[test]
    fn oversized_input_is_rejected_without_slicing() {
        let policy = TokenPolicy {
            max_input_tokens: 40,
            ..TokenPolicy::default()
        };
        let input = "요구사항".repeat(20);
        assert!(policy.check_input("system", &input).is_err());
        assert_eq!(input, "요구사항".repeat(20));
        assert!(estimate_tokens("한국어") > estimate_tokens("abc"));
    }
}
