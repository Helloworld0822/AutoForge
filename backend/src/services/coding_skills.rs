use crate::error::{AutoForgeError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingSkill {
    TargetedRead,
    MinimalDiff,
    ConciseOutput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodingSkills {
    #[serde(default)]
    pub enabled: Vec<CodingSkill>,
    #[serde(default)]
    pub custom_markdown: String,
}

impl CodingSkills {
    pub fn validate(&self) -> Result<()> {
        if self.enabled.len() > 3 || self.custom_markdown.chars().count() > 4000 {
            return Err(AutoForgeError::BadRequest(
                "스킬은 3개, 사용자 지침은 4,000자까지 적용할 수 있습니다.".into(),
            ));
        }
        Ok(())
    }

    pub fn prompt_note(&self) -> String {
        let mut notes = Vec::new();
        for skill in &self.enabled {
            notes.push(match skill {
                CodingSkill::TargetedRead => "Search symbols first; read only relevant files and ranges. Reuse findings instead of rereading entire files or dumping logs.",
                CodingSkill::MinimalDiff => "Reuse existing helpers and dependencies. Make the smallest complete change; avoid unrelated refactors and speculative abstractions. Preserve validation, security and required tests.",
                CodingSkill::ConciseOutput => "Keep progress and final explanations concise. Summarize command failures with relevant lines; do not repeat source code or full command output. Preserve the required structured output schema.",
            });
        }
        if !self.custom_markdown.trim().is_empty() {
            notes.push(self.custom_markdown.trim());
        }
        if notes.is_empty() {
            String::new()
        } else {
            format!(
                "\nCoding skills (preserve task requirements and quality gates):\n{}\n",
                notes.join("\n")
            )
        }
    }
}

pub async fn load_override(base: &Path, id: Uuid) -> Result<Option<CodingSkills>> {
    match tokio::fs::read(base.join(format!("projects/{id}/coding-skills.json"))).await {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|e| AutoForgeError::Store(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AutoForgeError::Store(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_empty_selection_and_rejects_oversize_custom_skill() {
        assert!(CodingSkills::default().validate().is_ok());
        let skills = CodingSkills {
            enabled: vec![],
            custom_markdown: "가".repeat(4001),
        };
        assert!(skills.validate().is_err());
    }

    #[tokio::test]
    async fn explicit_empty_override_disables_initial_skills() {
        let dir = std::env::temp_dir().join(format!("autoforge-skills-{}", Uuid::new_v4()));
        let id = Uuid::new_v4();
        assert!(load_override(&dir, id).await.unwrap().is_none());
        super::super::artifacts::save_json(
            &dir,
            &format!("projects/{id}/coding-skills.json"),
            &CodingSkills::default(),
        )
        .await
        .unwrap();
        let loaded = load_override(&dir, id).await.unwrap().unwrap();
        assert!(loaded.enabled.is_empty());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
