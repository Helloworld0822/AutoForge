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

/// Auto 모드에서 LLM이 구현 언어를 고를 때 참고할 종합 기준.
pub fn language_auto_selection_criteria() -> &'static str {
    "구현 언어 선택 시 아래 요소를 종합적으로 고려하세요:\n\
     - 예상 사용자·트래픽 규모: 소규모 MVP는 개발 속도 우선, 대규모·고동시성은 런타임 효율·수평 확장성 우선\n\
     - 서버 성능·인프라 비용: 제한된 리소스면 경량 런타임(go/rust), 여유 있으면 생산성 언어 고려\n\
     - 배포·운영 환경: 컨테이너·클라우드, 온프레미스, 모바일 네이티브 등\n\
     - 도메인·플랫폼: 웹 프론트엔드(typescript), iOS(swift), Android(kotlin), 시스템·임베디드(rust) 등\n\
     - 한국 사회·산업 맥락:\n\
       · 공공·정부·금융·대기업 레거시 연동·인력 풀 → java(Spring) 가점\n\
       · 스타트업·빠른 출시·프로토타입·AI/데이터 파이프라인 → python 가점\n\
       · B2C 웹·SaaS 풀스택 → typescript(node) 가점\n\
       · 고성능 API·마이크로서비스·인프라 도구 → go 또는 rust 가점\n\
     - language_rationale에 위 요소 중 어떤 판단이 결정적이었는지 1~2문장으로 명시하세요.\n"
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
                format!(
                    "프로젝트 요구사항을 분석하여 최적의 구현 언어를 선택하고 programming_language 필드에 명시하세요.\n\
                     {}",
                    language_auto_selection_criteria()
                )
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
