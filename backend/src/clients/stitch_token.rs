use crate::error::{AutoForgeError, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const TOKEN_REFRESH_BUFFER: Duration = Duration::from_secs(5 * 60);
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at: SystemTime,
}

#[derive(Clone)]
pub struct StitchTokenProvider {
    http: Client,
    static_token: Option<String>,
    adc_path: Option<PathBuf>,
    quota_project: Option<String>,
    allow_gcloud_fallback: bool,
    cache: Arc<Mutex<Option<CachedToken>>>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct AdcCredentials {
    #[serde(rename = "type")]
    cred_type: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
    quota_project_id: Option<String>,
    client_email: Option<String>,
    private_key: Option<String>,
    token_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    exp: Option<u64>,
}

pub fn resolve_gcloud_config_project() -> Option<String> {
    let output = std::process::Command::new("gcloud")
        .args(["config", "get-value", "project", "--quiet"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let project = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if project.is_empty() || project == "(unset)" {
        None
    } else {
        Some(project)
    }
}

pub fn is_token_expired(token: &str) -> bool {
    token_expiry(token).is_some_and(|exp| SystemTime::now() + Duration::from_secs(60) >= exp)
}

fn token_expiry(token: &str) -> Option<SystemTime> {
    let payload_b64 = token.split('.').nth(1)?;
    let mut padded = payload_b64.to_string();
    let pad = (4 - padded.len() % 4) % 4;
    padded.extend(std::iter::repeat_n('=', pad));
    let bytes = base64_decode_url(&padded)?;
    let claims: JwtClaims = serde_json::from_slice(&bytes).ok()?;
    let exp = claims.exp?;
    Some(UNIX_EPOCH + Duration::from_secs(exp))
}

fn base64_decode_url(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 256] = &{
        let mut t = [255u8; 256];
        let mut i = 0u8;
        while i < 26 {
            t[(b'A' + i) as usize] = i;
            t[(b'a' + i) as usize] = i + 26;
            i += 1;
        }
        let mut d = 0u8;
        while d < 10 {
            t[(b'0' + d) as usize] = d + 52;
            d += 1;
        }
        t[b'-' as usize] = 62;
        t[b'_' as usize] = 63;
        t
    };

    let bytes: Vec<u8> = input.bytes().filter(|b| *b != b'=').collect();
    if bytes.is_empty() || bytes.len() % 4 == 1 {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in bytes {
        let val = TABLE[b as usize];
        if val == 255 {
            return None;
        }
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

impl StitchTokenProvider {
    pub fn from_env(static_token: String, quota_project: Option<String>) -> Self {
        Self::build(static_token, resolve_adc_path(), quota_project, true)
    }

    #[cfg(test)]
    pub fn static_only(token: Option<String>) -> Self {
        Self::build(token.unwrap_or_default(), None, None, false)
    }

    fn build(
        static_token: String,
        adc_path: Option<PathBuf>,
        quota_project: Option<String>,
        allow_gcloud_fallback: bool,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("stitch token http client");

        let static_token = if static_token.trim().is_empty() {
            None
        } else if is_token_expired(static_token.trim()) {
            tracing::warn!(
                "ignoring expired STITCH_ACCESS_TOKEN from environment — using ADC/gcloud instead"
            );
            None
        } else {
            Some(static_token.trim().to_string())
        };

        let resolved_project = quota_project.or_else(|| {
            adc_path
                .as_deref()
                .and_then(StitchTokenProvider::quota_project_from_path)
        });
        let resolved_project = if allow_gcloud_fallback {
            resolved_project.or_else(resolve_gcloud_config_project)
        } else {
            resolved_project
        };

        Self {
            http,
            static_token,
            adc_path,
            quota_project: resolved_project,
            allow_gcloud_fallback,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn quota_project(&self) -> Option<&str> {
        self.quota_project.as_deref()
    }

    pub fn quota_project_from_adc(&self) -> Option<String> {
        self.adc_path
            .as_deref()
            .and_then(StitchTokenProvider::quota_project_from_path)
    }

    fn quota_project_from_path(path: &Path) -> Option<String> {
        let raw = std::fs::read_to_string(path).ok()?;
        let creds: AdcCredentials = serde_json::from_str(&raw).ok()?;
        creds.quota_project_id
    }

    pub fn can_provide_token(&self) -> bool {
        self.static_token.is_some()
            || self.adc_path.is_some()
            || (self.allow_gcloud_fallback && gcloud_available())
    }

    pub async fn access_token(&self, force_refresh: bool) -> Result<String> {
        if !force_refresh {
            if let Some(token) = self.cached_token().await {
                return Ok(token);
            }
        }

        let refreshed = self.refresh_token().await?;
        let mut cache = self.cache.lock().await;
        *cache = Some(refreshed.clone());
        Ok(refreshed.value)
    }

    async fn cached_token(&self) -> Option<String> {
        let cache = self.cache.lock().await;
        let cached = cache.as_ref()?;
        if SystemTime::now() + TOKEN_REFRESH_BUFFER >= cached.expires_at {
            return None;
        }
        Some(cached.value.clone())
    }

    async fn refresh_token(&self) -> Result<CachedToken> {
        if let Some(path) = &self.adc_path {
            match self.refresh_from_adc(path).await {
                Ok(token) => return Ok(token),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "failed to refresh Stitch token from ADC; trying fallbacks"
                    );
                }
            }
        }

        if self.allow_gcloud_fallback {
            if let Ok(token) = self.refresh_from_gcloud().await {
                return Ok(token);
            }
        }

        if let Some(static_token) = &self.static_token {
            if is_token_expired(static_token) {
                return Err(AutoForgeError::StitchApi(
                    "STITCH_ACCESS_TOKEN is expired — remove it from .env and use \
                     `gcloud auth application-default login`, or print a fresh token"
                        .into(),
                ));
            }
            tracing::debug!("using static STITCH_ACCESS_TOKEN (no expiry in JWT)");
            return Ok(CachedToken {
                value: static_token.clone(),
                expires_at: token_expiry(static_token)
                    .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(55 * 60)),
            });
        }

        Err(AutoForgeError::StitchApi(
            "Stitch Bearer token unavailable — run `gcloud auth application-default login`, \
             set GOOGLE_CLOUD_PROJECT, mount ADC into worker containers, or set a fresh \
             STITCH_ACCESS_TOKEN"
                .into(),
        ))
    }

    async fn refresh_from_adc(&self, path: &Path) -> Result<CachedToken> {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| AutoForgeError::StitchApi(format!("read ADC {}: {e}", path.display())))?;
        let creds: AdcCredentials = serde_json::from_str(&raw)
            .map_err(|e| AutoForgeError::StitchApi(format!("parse ADC {}: {e}", path.display())))?;

        match creds.cred_type.as_str() {
            "authorized_user" => self.refresh_authorized_user(&creds).await,
            "service_account" => self.refresh_service_account(&creds).await,
            other => Err(AutoForgeError::StitchApi(format!(
                "unsupported ADC type in {}: {other}",
                path.display()
            ))),
        }
    }

    async fn refresh_authorized_user(&self, creds: &AdcCredentials) -> Result<CachedToken> {
        let client_id = creds.client_id.as_deref().ok_or_else(|| {
            AutoForgeError::StitchApi("ADC authorized_user missing client_id".into())
        })?;
        let client_secret = creds.client_secret.as_deref().ok_or_else(|| {
            AutoForgeError::StitchApi("ADC authorized_user missing client_secret".into())
        })?;
        let refresh_token = creds.refresh_token.as_deref().ok_or_else(|| {
            AutoForgeError::StitchApi("ADC authorized_user missing refresh_token".into())
        })?;

        let mut req = self.http.post(GOOGLE_TOKEN_URL).form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ]);
        if let Some(project) = self.quota_project.as_deref() {
            req = req.header("X-Goog-User-Project", project);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        self.parse_token_response(resp).await
    }

    async fn refresh_service_account(&self, creds: &AdcCredentials) -> Result<CachedToken> {
        let client_email = creds.client_email.as_deref().ok_or_else(|| {
            AutoForgeError::StitchApi("ADC service_account missing client_email".into())
        })?;
        let private_key = creds.private_key.as_deref().ok_or_else(|| {
            AutoForgeError::StitchApi("ADC service_account missing private_key".into())
        })?;
        let token_uri = creds.token_uri.as_deref().unwrap_or(GOOGLE_TOKEN_URL);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?
            .as_secs() as i64;

        let claims = serde_json::json!({
            "iss": client_email,
            "sub": client_email,
            "aud": token_uri,
            "iat": now,
            "exp": now + 3600,
            "scope": CLOUD_PLATFORM_SCOPE,
        });

        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_rsa_pem(private_key.as_bytes())
                .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?,
        )
        .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        let mut req = self.http.post(token_uri).form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", jwt.as_str()),
        ]);
        if let Some(project) = self.quota_project.as_deref() {
            req = req.header("X-Goog-User-Project", project);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        self.parse_token_response(resp).await
    }

    async fn refresh_from_gcloud(&self) -> Result<CachedToken> {
        if !gcloud_available() {
            return Err(AutoForgeError::StitchApi("gcloud CLI not found".into()));
        }

        let mut cmd = tokio::process::Command::new("gcloud");
        cmd.args([
            "auth",
            "application-default",
            "print-access-token",
            "--quiet",
        ]);
        if let Some(project) = &self.quota_project {
            cmd.args(["--project", project]);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| AutoForgeError::StitchApi(format!("gcloud failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AutoForgeError::StitchApi(format!(
                "gcloud print-access-token failed: {stderr}"
            )));
        }

        let token = String::from_utf8(output.stdout)
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?
            .trim()
            .to_string();

        if token.is_empty() {
            return Err(AutoForgeError::StitchApi(
                "gcloud returned empty access token".into(),
            ));
        }

        Ok(CachedToken {
            value: token.clone(),
            expires_at: token_expiry(&token)
                .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(55 * 60)),
        })
    }

    async fn parse_token_response(&self, resp: reqwest::Response) -> Result<CachedToken> {
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AutoForgeError::StitchApi(format!(
                "Google token refresh failed ({status}): {body}"
            )));
        }

        let token: TokenResponse = resp
            .json()
            .await
            .map_err(|e| AutoForgeError::StitchApi(e.to_string()))?;

        Ok(CachedToken {
            value: token.access_token.clone(),
            expires_at: token_expiry(&token.access_token)
                .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(token.expires_in)),
        })
    }
}

fn resolve_adc_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())?;
    let default = PathBuf::from(home).join(".config/gcloud/application_default_credentials.json");
    default.is_file().then_some(default)
}

fn gcloud_available() -> bool {
    std::process::Command::new("gcloud")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_token_enables_bearer() {
        let provider = StitchTokenProvider::from_env("ya29.static-not-a-jwt".into(), None);
        assert!(provider.can_provide_token());
    }

    #[test]
    fn detects_expired_jwt() {
        // exp=1 (1970)
        let token = "aaa.eyJleHAiOjF9.signature";
        assert!(is_token_expired(token));
    }

    #[test]
    fn empty_static_without_adc_reports_unavailable_when_no_gcloud() {
        let provider = StitchTokenProvider::from_env(String::new(), None);
        if !gcloud_available() && provider.adc_path.is_none() {
            assert!(!provider.can_provide_token());
        }
    }
}
