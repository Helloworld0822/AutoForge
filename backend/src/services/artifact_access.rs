use crate::domain::ArtifactRef;
use crate::error::{AutoForgeError, Result};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// coder 입력 아티팩트 토큰의 유효 시간 (초)
pub const CODER_TOKEN_TTL_SECS: i64 = 1800;

const CODER_PURPOSE: &str = "coder_input";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoderArtifactClaims {
    pub purpose: String,
    pub project_id: String,
    pub key: String,
    pub exp: i64,
}

/// coder 프롬프트에 노출해도 되는 생성 산출물인지 판별한다.
/// 원문 PDF, 추출 텍스트, DevOps 원본, 내부 상태는 포함하지 않는다.
pub fn is_coder_artifact_allowed(name: &str) -> bool {
    matches!(
        name,
        "architecture.md" | "spec.md" | "tasks.json" | "project_spec.json" | "figma-design.json"
    ) || name.starts_with("screens/")
        || name.ends_with(".png")
        || name.ends_with(".html")
}

pub fn issue_coder_url(
    secret: &str,
    public_url: &str,
    project_id: Uuid,
    artifact: &ArtifactRef,
    now: DateTime<Utc>,
) -> Result<String> {
    if !is_coder_artifact_allowed(&artifact.name) {
        return Err(AutoForgeError::BadRequest(format!(
            "artifact {} is not allowed for coder delivery",
            artifact.name
        )));
    }
    let claims = CoderArtifactClaims {
        purpose: CODER_PURPOSE.into(),
        project_id: project_id.to_string(),
        key: artifact.key.clone(),
        exp: now.timestamp() + CODER_TOKEN_TTL_SECS,
    };
    let token = sign(secret, &claims)?;
    Ok(format!(
        "{}/artifacts/coder?token={token}",
        public_url.trim_end_matches('/')
    ))
}

pub fn verify_coder_token(
    secret: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<CoderArtifactClaims> {
    let (payload_hex, signature_hex) = token
        .split_once('.')
        .ok_or_else(|| AutoForgeError::BadRequest("malformed artifact token".into()))?;
    let payload = hex::decode(payload_hex)
        .map_err(|_| AutoForgeError::BadRequest("malformed artifact token payload".into()))?;
    let signature = hex::decode(signature_hex)
        .map_err(|_| AutoForgeError::BadRequest("malformed artifact token signature".into()))?;

    let expected = mac(secret, &payload)?;
    if expected.as_slice().ct_eq(signature.as_slice()).unwrap_u8() != 1 {
        return Err(AutoForgeError::BadRequest(
            "artifact token signature mismatch".into(),
        ));
    }

    let claims: CoderArtifactClaims = serde_json::from_slice(&payload)
        .map_err(|_| AutoForgeError::BadRequest("malformed artifact token claims".into()))?;

    if claims.purpose != CODER_PURPOSE {
        return Err(AutoForgeError::BadRequest(
            "artifact token purpose mismatch".into(),
        ));
    }
    if claims.exp < now.timestamp() {
        return Err(AutoForgeError::BadRequest("artifact token expired".into()));
    }
    Ok(claims)
}

fn sign(secret: &str, claims: &CoderArtifactClaims) -> Result<String> {
    let payload = serde_json::to_vec(claims)
        .map_err(|e| AutoForgeError::Internal(format!("artifact token encode: {e}")))?;
    let signature = mac(secret, &payload)?;
    Ok(format!(
        "{}.{}",
        hex::encode(&payload),
        hex::encode(&signature)
    ))
}

fn mac(secret: &str, message: &[u8]) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| AutoForgeError::Internal(format!("artifact token key: {e}")))?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// 아티팩트 텍스트에서 자격증명으로 보이는 패턴을 감지한다.
/// 전체 시크릿 탐지를 보장할 수 없으므로, 의심 시 노출하지 않는 fail-closed 용도다.
pub fn looks_like_secret(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "BEGIN RSA PRIVATE KEY",
        "BEGIN OPENSSH PRIVATE KEY",
        "BEGIN PRIVATE KEY",
        "AKIA",
        "ghp_",
        "xoxb-",
        "sk-",
        "password=",
        "secret_key",
    ];
    MARKERS.iter().any(|marker| text.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(name: &str) -> ArtifactRef {
        ArtifactRef {
            name: name.into(),
            key: format!("projects/p/{name}"),
            uri: String::new(),
            content_type: "text/markdown".into(),
            sha256: None,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn issues_and_verifies_a_coder_url() {
        let project_id = Uuid::new_v4();
        let url = issue_coder_url(
            "secret",
            "https://forge.example",
            project_id,
            &artifact("architecture.md"),
            now(),
        )
        .expect("issue");
        assert!(url.starts_with("https://forge.example/artifacts/coder?token="));
        let token = url.split("token=").nth(1).expect("token");
        let claims = verify_coder_token("secret", token, now()).expect("verify");
        assert_eq!(claims.project_id, project_id.to_string());
        assert_eq!(claims.key, "projects/p/architecture.md");
    }

    #[test]
    fn rejects_expired_wrong_secret_and_tampered_tokens() {
        let project_id = Uuid::new_v4();
        let url = issue_coder_url(
            "secret",
            "https://forge.example",
            project_id,
            &artifact("spec.md"),
            now(),
        )
        .expect("issue");
        let token = url.split("token=").nth(1).expect("token");

        let later = now() + chrono::Duration::seconds(CODER_TOKEN_TTL_SECS + 1);
        assert!(verify_coder_token("secret", token, later).is_err());
        assert!(verify_coder_token("other-secret", token, now()).is_err());

        let tampered = format!("{token}aa");
        assert!(verify_coder_token("secret", &tampered, now()).is_err());
    }

    #[test]
    fn rejects_disallowed_artifacts() {
        let project_id = Uuid::new_v4();
        for name in [
            "plan.pdf",
            "raw_text.md",
            "devops_raw_text.md",
            "usage.json",
        ] {
            assert!(issue_coder_url(
                "secret",
                "https://forge.example",
                project_id,
                &artifact(name),
                now(),
            )
            .is_err());
        }
    }

    #[test]
    fn detects_obvious_secret_markers() {
        assert!(looks_like_secret("token ghp_abcdef"));
        assert!(looks_like_secret("-----BEGIN PRIVATE KEY-----"));
        assert!(!looks_like_secret("# Architecture\nUse Rust."));
    }
}
