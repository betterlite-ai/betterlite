use std::error::Error;
use std::sync::Arc;

use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary};

mod counters;
pub mod input;
pub mod ports;
pub mod records;
pub mod rejections;
mod tests;

use input::{BatchLimitCounterCall, BatchLimitCountersInput};
use ports::{BatchCounterLimiter, BatchCounterStash, BatchEnqueuedSource, BatchWindowCache};
use records::{BatchLimitCounterAnswer, BatchLimitCountersResult, EpochClock};
use rejections::{BatchCounterValueError, RateLimitRejection};

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

// The batch rate limiter's counter half: the clock, the three repositories, the request stash and the provider resolver
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

impl<Limiter, Store, Cache> BatchLimitCounters<Limiter, Store, Cache>
where
    Limiter: BatchCounterLimiter,
    Store: BatchEnqueuedSource,
    Cache: BatchWindowCache,
{
    pub async fn run(
        &self,
        input: &BatchLimitCountersInput,
    ) -> Result<
        BatchLimitCountersResult,
        BatchLimitCountersError<Limiter::Error, Store::Error, Cache::Error>,
    > {
        self.limits.check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: input.calls.len(),
        }))?;
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(input.principal.0.clone()))?;
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let answer = self.answer(&auth, call).await?;
            let ends_run = answer.is_rejection();
            answers.push(answer);
            if ends_run {
                break;
            }
        }
        Ok(BatchLimitCountersResult { answers })
    }

    // one arm per owned batch callable; a rejection ends the request where it happens
    #[expect(clippy::too_many_lines, reason = "one arm per owned batch callable")]
    async fn answer(
        &self,
        auth: &UserAPIKeyAuth,
        call: &BatchLimitCounterCall,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        match call {
            BatchLimitCounterCall::ReserveBatchEnqueuedTokens {
                data,
                batch_usage,
                scopes,
            } => {
                self.reserve_enqueued_tokens(data, batch_usage, scopes)
                    .await
            }
            BatchLimitCounterCall::RaiseEnqueuedLimitError {
                over_limit,
                data,
                batch_usage,
            } => self.enqueued_rejection(over_limit, data, batch_usage),
            BatchLimitCounterCall::RaiseRateLimitError {
                status,
                descriptors,
                batch_usage,
                limit_type,
                requested_model,
                window_start,
            } => {
                let facts = RateLimitRejection {
                    status,
                    descriptors,
                    batch_usage,
                    limit_type,
                    requested_model: requested_model.as_deref(),
                    window_start: *window_start,
                    now: (self.clock)(),
                    window_size: self.window_size,
                };
                self.rate_rejection(&facts)
            }
            BatchLimitCounterCall::CheckAndIncrementBatchCounters {
                data,
                batch_usage,
                descriptors,
            } => {
                self.check_and_increment(auth, data, batch_usage, descriptors.as_deref())
                    .await
            }
            BatchLimitCounterCall::ReadTpdWindowStart { status } => {
                self.read_window_start(status).await
            }
            BatchLimitCounterCall::BuildTpdRefundOps {
                descriptors,
                tokens,
                reservation_windows,
            } => self.refund_operations(descriptors, *tokens, reservation_windows),
        }
    }
}
