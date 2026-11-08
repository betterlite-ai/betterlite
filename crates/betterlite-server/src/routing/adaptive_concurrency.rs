use std::sync::Arc;
use std::time::Duration;

use betterlite_router::adaptive_concurrency::error::AcquireError;
use betterlite_router::adaptive_concurrency::{AdaptiveConcurrencyLimiter, Permit, PutSample};
use tokio::time::Instant;

mod tests;

// one integration callback's limiter, and the longest wait its uploads may take for a permit
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

    pub fn record(&self, sample: PutSample) {
        let recorded = self.limiter.snapshot().limit;
        self.limiter.record(sample);
        let limit = self.limiter.snapshot().limit;
        if limit != recorded {
            tracing::info!(limit, "the integration adaptive concurrency limit changed");
        }
    }
}
