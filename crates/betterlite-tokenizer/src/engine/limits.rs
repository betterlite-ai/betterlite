use crate::engine::Engine;
use crate::engine::error::EngineError;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
pub type CountJob = Box<dyn FnOnce() -> Result<usize, EngineError> + Send + 'static>;
pub type BuildJob = Box<dyn FnOnce() -> Result<Engine, EngineError> + Send + 'static>;
pub type ConvertJob = Box<dyn FnOnce() -> Result<Vec<u8>, EngineError> + Send + 'static>;
#[derive(Debug, Clone, Copy)]
pub struct EncodeLimits {
    pub input_bytes: usize,
    pub output_ids: usize,
}
impl Default for EncodeLimits {
    fn default() -> Self {
        Self {
            input_bytes: 16_000_000,
            output_ids: 16_000_000,
        }
    }
}
#[async_trait]
pub trait BlockingCounter: Send + Sync {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError>;
    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError>;
    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError>;
}
// LiteLLM's anyio CapacityLimiter: the permit travels with the job so a cancelled caller cannot free it early
pub struct CountingService {
    permits: Arc<Semaphore>,
    facility: Arc<dyn BlockingCounter>,
    limits: EncodeLimits,
}
impl CountingService {
    #[must_use]
    pub fn new(
        max_concurrent: usize,
        facility: Arc<dyn BlockingCounter>,
        limits: EncodeLimits,
    ) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
            facility,
            limits,
        }
    }
    pub async fn count(&self, engine: &Engine, text: &str) -> Result<usize, EngineError> {
        if text.len() > self.limits.input_bytes {
            return Err(EngineError::InputTooLarge {
                bytes: text.len(),
                limit: self.limits.input_bytes,
            });
        }
        let permit = self.permit().await?;
        let (owned_engine, owned_text) = (engine.clone(), text.to_owned());
        let limit = self.limits.output_ids;
        let job: CountJob = Box::new(move || {
            let _held = permit;
            let ids = owned_engine.encode(&owned_text)?;
            if ids.len() > limit {
                return Err(EngineError::OutputTooLarge {
                    ids: ids.len(),
                    limit,
                });
            }
            Ok(ids.len())
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
    pub async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        let permit = self.permit().await?;
        let wrapped: ConvertJob = Box::new(move || {
            let _held = permit;
            job()
        });
        self.facility.convert(wrapped).await
    }
    async fn permit(&self) -> Result<OwnedSemaphorePermit, EngineError> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| EngineError::PoolClosed)
    }
}
