use std::error::Error;
use std::sync::Arc;

use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::AuthError;

use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

mod charges;
mod counters;
pub mod input;
pub mod ports;
pub mod records;
pub mod rejections;
mod tests;

use ports::{BatchCounterLimiter, BatchCounterStash, BatchEnqueuedSource, BatchWindowCache};
use records::EpochClock;
use rejections::BatchCounterValueError;

// Every way the counter work fails before or around a rejection; the 429s themselves come back as answers
#[derive(Debug, thiserror::Error)]
pub enum BatchLimitCountersError<LimiterError, StoreError, CacheError>
where
    LimiterError: Error + 'static,
    StoreError: Error + 'static,
    CacheError: Error + 'static,
{
    #[error(transparent)]
    Principal(#[from] AuthError),
    #[error(transparent)]
    Boundary(#[from] ApiLimitError),
    #[error(transparent)]
    Rejection(#[from] BatchCounterValueError),
    #[error("the batch counter limiter failed")]
    Limits(#[source] LimiterError),
    #[error("the enqueued-token store failed")]
    Store(#[source] StoreError),
    #[error("the counter window cache failed")]
    Window(#[source] CacheError),
}

pub(crate) type CountingFailure<Limiter, Store, Cache> = BatchLimitCountersError<
    <Limiter as BatchCounterLimiter>::Error,
    <Store as BatchEnqueuedSource>::Error,
    <Cache as BatchWindowCache>::Error,
>;

pub struct BatchLimitCounters<Limiter, Store, Cache> {
    pub clock: EpochClock,
    pub limiter: Limiter,
    pub store: Store,
    pub cache: Cache,
    pub stash: Arc<dyn BatchCounterStash>,
    pub resolver: Arc<dyn ProviderResolution>,
    pub router: Option<Arc<dyn DeploymentSnapshot>>,
    pub window_size: i64,
    pub limits: ApiLimits,
}
