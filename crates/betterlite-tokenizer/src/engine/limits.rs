use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::engine::Engine;
use crate::engine::error::EngineError;

pub type CountJob = Box<dyn FnOnce() -> Result<usize, EngineError> + Send + 'static>;

#[async_trait]
pub trait BlockingCounter: Send + Sync {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError>;
}

#[derive(Debug, Default)]
pub struct JoinSetCounter;

#[async_trait]
impl BlockingCounter for JoinSetCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(job);
        match tasks.join_next().await {
            Some(Ok(counted)) => counted,
            Some(Err(error)) => Err(EngineError::Task(Box::new(error))),
            None => Err(EngineError::NoResult),
        }
    }
}

// LiteLLM's anyio CapacityLimiter: counts run on a blocking task, so the permit travels with the job and a cancelled caller cannot free it early
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
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| EngineError::PoolClosed)?;
        let owned_engine = engine.clone();
        let owned_text = text.to_owned();
        let job: CountJob = Box::new(move || {
            let _held = permit;
            owned_engine.encode(&owned_text).map(|ids| ids.len())
        });
        self.facility.count(job).await
    }
}
