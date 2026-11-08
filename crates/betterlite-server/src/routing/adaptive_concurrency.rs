use std::sync::Arc;
use std::time::Duration;

use betterlite_router::adaptive_concurrency::error::AcquireError;
use betterlite_router::adaptive_concurrency::{AdaptiveConcurrencyLimiter, Permit, PutSample};
use tokio::time::Instant;

mod tests;

#[derive(Clone)]
pub struct IntegrationConcurrency {
    pub limiter: Arc<AdaptiveConcurrencyLimiter>,
    pub wait_timeout: Duration,
}

impl IntegrationConcurrency {
    pub async fn permit(&self) -> Result<Permit, AcquireError> {
        let deadline = Instant::now()
            .checked_add(self.wait_timeout)
            .ok_or(AcquireError::DeadlineExceeded)?;
        self.limiter.acquire(deadline).await
    }

    #[must_use]
    pub fn record(&self, sample: PutSample) -> u64 {
        self.limiter.record(sample);
        self.limiter.snapshot().limit
    }
}
