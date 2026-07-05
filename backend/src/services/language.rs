use crate::domain::{LanguageMode, ProgrammingLanguage};
use serde_json::Value;

/// 요약 JSON·사용자 지정·기본값 순으로 구현 언어를 결정한다.
pub fn resolve_effective_language(
    mode: LanguageMode,
    user_choice: Option<ProgrammingLanguage>,
    summary_text: &str,
) -> ProgrammingLanguage {
    if mode == LanguageMode::Manual {
        if let Some(lang) = user_choice {
            return lang;
        }
    }

    if let Some(lang) = parse_language_from_summary(summary_text) {
        return lang;
    }

    if let Some(lang) = user_choice {
        return lang;
    }

    ProgrammingLanguage::TypeScript
}

pub fn parse_language_from_summary(summary_text: &str) -> Option<ProgrammingLanguage> {
    let value: Value = serde_json::from_str(summary_text).ok()?;
    let raw = value
        .get("programming_language")
        .or_else(|| value.get("recommended_language"))
        .and_then(|v| v.as_str())?;
    ProgrammingLanguage::from_str_loose(raw)
}

pub fn language_prompt_note(
    mode: LanguageMode,
    user_choice: Option<ProgrammingLanguage>,
    resolved: Option<ProgrammingLanguage>,
) -> String {
    match mode {
        LanguageMode::Manual => {
            let lang = user_choice
                .or(resolved)
                .unwrap_or(ProgrammingLanguage::TypeScript);
            format!(
                "주 구현 언어는 반드시 {} ({})를 사용하세요.\n",
                lang.label(),
                lang.as_str()
            )
        }
        LanguageMode::Auto => {
            if let Some(lang) = resolved {
                format!(
                    "요약 분석 결과 권장 구현 언어는 {} ({})입니다. 이 언어를 기본으로 사용하세요.\n",
                    lang.label(),
                    lang.as_str()
                )
            } else {
                "프로젝트 요구사항을 분석하여 최적의 구현 언어를 선택하고 programming_language 필드에 명시하세요.\n".into()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_mode_prefers_user_choice() {
        let lang = resolve_effective_language(
            LanguageMode::Manual,
            Some(ProgrammingLanguage::Rust),
            r#"{"programming_language":"python"}"#,
        );
        assert_eq!(lang, ProgrammingLanguage::Rust);
    }

    #[test]
    fn auto_mode_uses_summary_language() {
        let lang = resolve_effective_language(
            LanguageMode::Auto,
            None,
            r#"{"programming_language":"go"}"#,
        );
        assert_eq!(lang, ProgrammingLanguage::Go);
    }
}
