use std::convert::Infallible;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;

use crate::database_refresh::handle::PoolFuture;

pub trait ReconnectLock: Send + Sync {
    type Error: Display + Send + Sync + 'static;
    type Hold: Send + 'static;

    fn acquire(&self, limit: Option<Duration>) -> PoolFuture<'_, Option<Self::Hold>, Self::Error>;
}

pub struct TokioLock {
    permits: Arc<Semaphore>,
}

impl Default for TokioLock {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(1)),
        }
    }
}

impl ReconnectLock for TokioLock {
    type Error = Infallible;
    type Hold = OwnedSemaphorePermit;

    fn acquire(&self, limit: Option<Duration>) -> PoolFuture<'_, Option<Self::Hold>, Infallible> {
        Box::pin(async move {
            let permits = Arc::clone(&self.permits);
            let acquired = match limit {
                Some(limit) => timeout(limit, permits.acquire_owned()).await.ok(),
                None => Some(permits.acquire_owned().await),
            };
            Ok(acquired.and_then(Result::ok))
        })
    }
}
