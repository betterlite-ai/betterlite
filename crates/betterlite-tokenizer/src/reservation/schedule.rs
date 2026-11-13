use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::error::ReservationError;

// LiteLLM's 30000-character threshold: at this size the accounting leaves the request's task
pub const OFFLOAD_MIN_CHARS: usize = 30_000;

pub type AccountJob = Pin<Box<dyn Future<Output = Counts> + Send + 'static>>;

pub type Counts = IndexMap<String, usize>;

#[async_trait]
pub trait AccountPool: Send + Sync {
    async fn run(&self, job: AccountJob) -> Result<Counts, ReservationError>;
}

pub struct BoundedPool {
    permits: Arc<Semaphore>,
}

impl BoundedPool {
    #[must_use]
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }
}

// B196 counting is async, so the accounting is a bounded task while each encode keeps the caller's blocking facility
#[async_trait]
impl AccountPool for BoundedPool {
    async fn run(&self, job: AccountJob) -> Result<Counts, ReservationError> {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| ReservationError::PoolClosed)?;
        let mut workers = JoinSet::new();
        workers.spawn(async move {
            let _held = permit;
            job.await
        });
        match workers.join_next().await {
            Some(Ok(counts)) => Ok(counts),
            Some(Err(source)) => Err(ReservationError::Worker { source }),
            None => Err(ReservationError::WorkerMissing),
        }
    }
}
