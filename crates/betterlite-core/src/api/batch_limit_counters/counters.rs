use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::api::api_boundaries::{ApiBoundariesInput, ItemsBoundary};
use crate::api::batch_enqueued_reserve::{BatchEnqueuedReserveResult, BatchEnqueuedTokenScope};
use crate::api::batch_limit_descriptors::descriptors::{BatchLimitDescriptor, TPD_SUFFIX};
use crate::api::request_destination::RequestData;

use super::charges::charged_counters;
use super::input::{BatchLimitCounterCall, BatchLimitCountersInput};
use super::ports::{BatchCounterLimiter, BatchEnqueuedSource, BatchWindowCache};
use super::records::{
    BatchCounterCode, BatchCounterResponse, BatchCounterStatus, BatchEnqueuedLimitExceeded,
    BatchFileUsage, BatchLimitCounterAnswer, BatchLimitCountersResult,
};
use super::rejections::refunds::refund_ops;
use super::rejections::{self, Attribution, RateLimitRejection};
use super::{BatchLimitCounters, BatchLimitCountersError, CountingFailure};

impl<Limiter, Store, Cache> BatchLimitCounters<Limiter, Store, Cache>
where
    Limiter: BatchCounterLimiter,
    Store: BatchEnqueuedSource,
    Cache: BatchWindowCache,
{
    pub(super) async fn reserve_enqueued_tokens(
        &self,
        data: &RequestData,
        batch_usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let outcome = self
            .store
            .reserve_tokens(batch_usage.total_tokens, scopes)
            .await
            .map_err(BatchLimitCountersError::Store)?;
        match outcome {
            BatchEnqueuedReserveResult::Reservation(reservation) => {
                self.stash.set_batch_enqueued_reservation(reservation);
                Ok(BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection: None })
            }
            BatchEnqueuedReserveResult::OverLimit(over_limit) => {
                let reached = BatchEnqueuedLimitExceeded {
                    scope: over_limit.scope,
                    enqueued: over_limit.enqueued,
                };
                let error = rejections::enqueued_error(
                    &reached,
                    data,
                    batch_usage,
                    &Attribution {
                        resolver: self.resolver.as_ref(),
                        router: self.router.as_deref(),
                    },
                )?;
                Ok(BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens {
                    rejection: Some(Box::new(error)),
                })
            }
        }
    }

    pub(super) fn rate_rejection(
        &self,
        facts: &RateLimitRejection<'_>,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let error = rejections::rate_limit_error(
            facts,
            &Attribution {
                resolver: self.resolver.as_ref(),
                router: self.router.as_deref(),
            },
        )?;
        Ok(BatchLimitCounterAnswer::RaiseRateLimitError {
            error: Box::new(error),
        })
    }

    pub(super) async fn check_and_increment(
        &self,
        auth: &UserAPIKeyAuth,
        data: &RequestData,
        batch_usage: &BatchFileUsage,
        descriptors: Option<&[BatchLimitDescriptor]>,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let (charged, increments) =
            charged_counters(&self.limiter, auth, data, batch_usage, descriptors)
                .map_err(BatchLimitCountersError::Limits)?;
        self.limits.check(ApiBoundariesInput::Items(ItemsBoundary {
            known_count: charged.len(),
        }))?;
        let response = self
            .limiter
            .atomic_check_and_increment_by_n(&charged, &increments)
            .await
            .map_err(BatchLimitCountersError::Limits)?;
        self.stash.set_batch_tpd_refund_ops(Vec::new());
        if let Some(error) = self
            .first_rejection(&charged, batch_usage, data, &response)
            .await?
        {
            return Ok(BatchLimitCounterAnswer::CheckAndIncrementBatchCounters {
                rejection: Some(Box::new(error)),
            });
        }
        // the limiter's windows scale with its descriptors, so a post-charge bound would reject a request LiteLLM completes
        let operations = refund_ops(
            &charged,
            batch_usage.total_tokens,
            &response.reservation_windows,
            &self.limiter,
        )?;
        self.stash.set_batch_tpd_refund_ops(operations);
        Ok(BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection: None })
    }

    async fn first_rejection(
        &self,
        charged: &[BatchLimitDescriptor],
        batch_usage: &BatchFileUsage,
        data: &RequestData,
        response: &BatchCounterResponse,
    ) -> Result<Option<ProxyRateLimitError>, CountingFailure<Limiter, Store, Cache>> {
        if response.overall_code != BatchCounterCode::OverLimit {
            return Ok(None);
        }
        let Some(status) = response
            .statuses
            .iter()
            .find(|status| status.code == BatchCounterCode::OverLimit)
        else {
            return Ok(None);
        };
        let window_start = self.tpd_window_start(status).await?;
        let facts = RateLimitRejection {
            status,
            descriptors: charged,
            batch_usage,
            limit_type: status.rate_limit_type.as_str(),
            requested_model: rejections::request_model(data),
            window_start,
            now: (self.clock)(),
            window_size: self.window_size,
        };
        Ok(Some(rejections::rate_limit_error(
            &facts,
            &Attribution {
                resolver: self.resolver.as_ref(),
                router: self.router.as_deref(),
            },
        )?))
    }

    pub(super) async fn tpd_window_start(
        &self,
        status: &BatchCounterStatus,
    ) -> Result<Option<i64>, CountingFailure<Limiter, Store, Cache>> {
        if !status.descriptor_key.ends_with(TPD_SUFFIX) {
            return Ok(None);
        }
        let value = status.descriptor_value.as_deref().unwrap_or("");
        let key = format!("{{{}:{value}}}:window", status.descriptor_key);
        let stored = self
            .cache
            .get(&key)
            .await
            .map_err(BatchLimitCountersError::Window)?;
        Ok(stored.and_then(|start| start.seconds()))
    }

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
            self.limits.check(ApiBoundariesInput::Items(ItemsBoundary {
                known_count: call.nested_items(),
            }))?;
            let answer = self.answer(&auth, call).await?;
            let ends_run = answer.is_rejection();
            answers.push(answer);
            if ends_run {
                break;
            }
        }
        Ok(BatchLimitCountersResult { answers })
    }

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
            } => {
                let error = rejections::enqueued_error(
                    over_limit,
                    data,
                    batch_usage,
                    &Attribution {
                        resolver: self.resolver.as_ref(),
                        router: self.router.as_deref(),
                    },
                )?;
                Ok(BatchLimitCounterAnswer::RaiseEnqueuedLimitError {
                    error: Box::new(error),
                })
            }
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
                let window_start = self.tpd_window_start(status).await?;
                Ok(BatchLimitCounterAnswer::ReadTpdWindowStart { window_start })
            }
            BatchLimitCounterCall::BuildTpdRefundOps {
                descriptors,
                tokens,
                reservation_windows,
            } => {
                let operations =
                    refund_ops(descriptors, *tokens, reservation_windows, &self.limiter)?;
                Ok(BatchLimitCounterAnswer::BuildTpdRefundOps { operations })
            }
        }
    }
}
