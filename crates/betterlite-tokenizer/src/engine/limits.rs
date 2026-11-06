use crate::engine::Engine;
use crate::engine::error::EngineError;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
pub type CountJob = Box<dyn FnOnce() -> Result<usize, EngineError> + Send + 'static>;
pub type BuildJob = Box<dyn FnOnce() -> Result<Engine, EngineError> + Send + 'static>;
#[async_trait]
pub trait BlockingCounter: Send + Sync {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError>;
    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError>;
}
// LiteLLM's anyio CapacityLimiter: the permit travels with the job so a cancelled caller cannot free it early
pub struct CountingService {
    permits: Arc<Semaphore>,
    facility: Arc<dyn BlockingCounter>,
}
impl CountingService {
    #[must_use]
    pub fn new(max_concurrent: usize, facility: Arc<dyn BlockingCounter>) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
            facility,
        }
    }
    pub async fn count(&self, engine: &Engine, text: &str) -> Result<usize, EngineError> {
        let permit = self.permit().await?;
        let (owned_engine, owned_text) = (engine.clone(), text.to_owned());
        let job: CountJob = Box::new(move || {
            let _held = permit;
            owned_engine.encode(&owned_text).map(|ids| ids.len())
        });
        self.facility.count(job).await
    }
    pub async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        let permit = self.permit().await?;
        let wrapped: BuildJob = Box::new(move || {
            let _held = permit;
            job()
        });
        self.facility.build(wrapped).await
    }
    async fn permit(&self) -> Result<OwnedSemaphorePermit, EngineError> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| EngineError::PoolClosed)
    }
}
