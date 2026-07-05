use crate::domain::ArchitectureClarification;
use crate::error::{AutoForgeError, Result};

/// LLM 응답에서 JSON 객체를 추출한다 (raw JSON, ```json 블록, 본문 내 첫 객체).
pub fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        return Some(v);
    }

    if let Some(start) = text.find("```json") {
        let rest = &text[start + 7..];
        if let Some(end) = rest.find("```") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest[..end].trim()) {
                return Some(v);
            }
        }
    }

    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            if end > start {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[start..=end]) {
                    return Some(v);
                }
            }
        }
    }

    None
}

pub fn parse_clarification_questions(text: &str) -> Result<Vec<ArchitectureClarification>> {
    let value = extract_json_object(text).ok_or_else(|| AutoForgeError::StageFailed {
        stage: crate::domain::StageId::Architect,
        message: "failed to parse architecture questions JSON".into(),
    })?;

    let questions = value
        .get("questions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AutoForgeError::StageFailed {
            stage: crate::domain::StageId::Architect,
            message: "questions array missing in architect draft response".into(),
        })?;

    let mut out = Vec::new();
    for (idx, item) in questions.iter().enumerate() {
        let question = item
            .get("text")
            .or_else(|| item.get("question"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if question.is_empty() {
            continue;
        }

        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("q{}", idx + 1));

        let options = item
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let required = item
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let category = item
            .get("category")
            .and_then(|v| v.as_str())
            .map(String::from);

        out.push(ArchitectureClarification {
            id,
            question,
            options,
            required,
            category,
            answer: None,
            answered_at: None,
        });
    }

    Ok(out)
}

pub fn all_required_answered(clarifications: &[ArchitectureClarification]) -> bool {
    clarifications
        .iter()
        .filter(|q| q.required)
        .all(|q| q.answer.as_ref().is_some_and(|a| !a.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_questions_from_json_block() {
        let text = r#"Here are questions:
```json
{"questions":[{"id":"db","text":"Which database?","options":["Postgres","MySQL"],"required":true}]}
```"#;
        let qs = parse_clarification_questions(text).unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].id, "db");
        assert_eq!(qs[0].options, vec!["Postgres", "MySQL"]);
    }
}
