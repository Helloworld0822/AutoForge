use crate::domain::Project;
use crate::error::Result;
use async_trait::async_trait;
use uuid::Uuid;

pub mod memory;
pub mod notifying;
pub mod redis_store;

pub use memory::MemoryStore;
pub use notifying::NotifyingStore;
pub use redis_store::RedisProjectStore;

#[async_trait]
pub trait ProjectStore: Send + Sync {
    async fn save(&self, project: &Project) -> Result<()>;
    async fn get(&self, id: Uuid) -> Result<Option<Project>>;
    async fn list(&self) -> Result<Vec<Project>>;
}

pub type ArcStore = std::sync::Arc<dyn ProjectStore>;
