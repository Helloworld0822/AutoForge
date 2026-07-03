use crate::domain::ArtifactRef;
use crate::error::{AutoForgeError, Result};
use async_trait::async_trait;
use bytes::Bytes;
use std::path::{Component, Path, PathBuf};

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<ArtifactRef>;
    async fn get(&self, key: &str) -> Result<Bytes>;
    fn uri_for(&self, key: &str) -> String;
    /// 로컬 디스크에 저장되어 프로세스 재시작에도 보존되는지 여부.
    /// 컨테이너 볼륨을 마운트하지 않은 경우에만 false가 될 수 있다.
    fn is_durable(&self) -> bool {
        true
    }
}

/// 파이프라인 산출물과 사용자가 업로드한 이미지를 로컬 디스크에 저장하는 스토어.
///
/// 별도의 S3/MinIO 같은 오브젝트 스토리지 없이도 동작하도록 설계되었다.
/// `ARTIFACTS_DIR`에 파일을 그대로 저장하며, Compose/Podman 환경에서는 이 경로를
/// 공유 볼륨으로 마운트해 api/worker/orchestrator 프로세스가 같은 파일을 보게 한다.
pub struct LocalArtifactStore {
    base_dir: PathBuf,
    public_url: String,
}

/// 이미지 호스팅 하위 디렉터리 (media/{filename})
pub const MEDIA_DIR: &str = "media";

#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaEntry {
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
}

impl LocalArtifactStore {
    pub fn new(base_dir: impl Into<PathBuf>, public_url: impl Into<String>) -> Result<Self> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| AutoForgeError::Artifacts(format!("cannot create {base_dir:?}: {e}")))?;
        std::fs::create_dir_all(base_dir.join(MEDIA_DIR))
            .map_err(|e| AutoForgeError::Artifacts(format!("cannot create media dir: {e}")))?;
        Ok(Self {
            base_dir,
            public_url: public_url.into().trim_end_matches('/').to_string(),
        })
    }

    /// 키를 안전한 로컬 경로로 변환한다. `..` 등 경로 탈출 시도는 거부한다.
    fn resolve(&self, key: &str) -> Result<PathBuf> {
        if key.is_empty() {
            return Err(AutoForgeError::BadRequest("empty artifact key".into()));
        }
        let path = Path::new(key);
        if path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(AutoForgeError::BadRequest(format!(
                "invalid artifact key: {key}"
            )));
        }
        Ok(self.base_dir.join(path))
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// `media/` 하위에 업로드된 이미지 목록을 최신순으로 반환한다.
    pub async fn list_media(&self) -> Result<Vec<MediaEntry>> {
        let dir = self.base_dir.join(MEDIA_DIR);
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| AutoForgeError::Artifacts(format!("cannot list media dir: {e}")))?;

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|e| AutoForgeError::Artifacts(e.to_string()))?
        {
            let meta = match entry.metadata().await {
                Ok(m) if m.is_file() => m,
                _ => continue,
            };
            let filename = entry.file_name().to_string_lossy().to_string();
            let uploaded_at = meta
                .modified()
                .ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
                .unwrap_or_else(chrono::Utc::now);
            entries.push(MediaEntry {
                url: format!("{}/media/{filename}", self.public_url),
                filename,
                size: meta.len(),
                uploaded_at,
            });
        }

        entries.sort_by_key(|e| std::cmp::Reverse(e.uploaded_at));
        Ok(entries)
    }
}

#[async_trait]
impl ArtifactStore for LocalArtifactStore {
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<ArtifactRef> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AutoForgeError::Artifacts(format!("mkdir {parent:?}: {e}")))?;
        }
        tokio::fs::write(&path, &data)
            .await
            .map_err(|e| AutoForgeError::Artifacts(format!("write {key}: {e}")))?;

        Ok(ArtifactRef {
            name: key.rsplit('/').next().unwrap_or(key).to_string(),
            key: key.to_string(),
            uri: self.uri_for(key),
            content_type: content_type.to_string(),
            sha256: None,
        })
    }

    async fn get(&self, key: &str) -> Result<Bytes> {
        let path = self.resolve(key)?;
        let data = tokio::fs::read(&path)
            .await
            .map_err(|_| AutoForgeError::Artifacts(format!("not found: {key}")))?;
        Ok(Bytes::from(data))
    }

    fn uri_for(&self, key: &str) -> String {
        if let Some(filename) = key.strip_prefix(&format!("{MEDIA_DIR}/")) {
            format!("{}/media/{filename}", self.public_url)
        } else {
            format!("{}/artifacts/{key}", self.public_url)
        }
    }
}

/// 파일 확장자로부터 이미지 MIME 타입을 추정한다. 매칭되는 확장자가 없으면
/// `application/octet-stream`을 반환한다.
pub fn guess_image_content_type(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// 매직 바이트로 이미지 파일 형식을 판별하고 확장자를 반환한다.
/// 지원하지 않는 형식이면 `None`.
pub fn detect_image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
        Some("svg")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> LocalArtifactStore {
        let dir = std::env::temp_dir().join(format!("autoforge-test-{}", uuid::Uuid::new_v4()));
        LocalArtifactStore::new(dir, "http://localhost").unwrap()
    }

    #[tokio::test]
    async fn put_and_get_roundtrip() {
        let store = temp_store();
        let artifact = store
            .put(
                "projects/abc/plan.pdf",
                Bytes::from_static(b"%PDF-1.4"),
                "application/pdf",
            )
            .await
            .unwrap();
        assert_eq!(artifact.key, "projects/abc/plan.pdf");
        assert_eq!(artifact.name, "plan.pdf");

        let bytes = store.get("projects/abc/plan.pdf").await.unwrap();
        assert_eq!(&bytes[..], b"%PDF-1.4");
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let store = temp_store();
        let result = store
            .put(
                "../../etc/passwd",
                Bytes::from_static(b"evil"),
                "text/plain",
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn media_uri_uses_media_route() {
        let store = temp_store();
        let artifact = store
            .put(
                "media/abc123.png",
                Bytes::from_static(b"\x89PNG\r\n\x1a\n"),
                "image/png",
            )
            .await
            .unwrap();
        assert_eq!(artifact.uri, "http://localhost/media/abc123.png");
    }

    #[test]
    fn detects_common_image_formats() {
        assert_eq!(detect_image_extension(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(detect_image_extension(b"\xff\xd8\xff\xe0"), Some("jpg"));
        assert_eq!(detect_image_extension(b"GIF89a"), Some("gif"));
        assert_eq!(detect_image_extension(b"not an image"), None);
    }
}
