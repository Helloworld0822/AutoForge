use super::{ArcStore, ProjectStore};
use crate::domain::Project;
use crate::error::Result;
use crate::services::project_watch::ProjectWatch;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

/// 저장 후 ProjectWatch 알림을 보내는 ProjectStore 래퍼.
pub struct NotifyingStore {
    inner: ArcStore,
    watch: Arc<ProjectWatch>,
}

impl NotifyingStore {
    pub fn new(inner: ArcStore, watch: Arc<ProjectWatch>) -> Self {
        Self { inner, watch }
    }
}

#[async_trait]
impl ProjectStore for NotifyingStore {
    async fn save(&self, project: &Project) -> Result<()> {
        self.inner.save(project).await?;
        self.watch.notify(project.id.0);
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Project>> {
        self.inner.get(id).await
    }

    async fn list(&self) -> Result<Vec<Project>> {
        self.inner.list().await
    }
}
