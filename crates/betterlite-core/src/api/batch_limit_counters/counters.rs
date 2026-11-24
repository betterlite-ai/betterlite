use std::collections::BTreeSet;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::api::batch_enqueued_reserve::{BatchEnqueuedReserveResult, BatchEnqueuedTokenScope};
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor, TPD_SUFFIX,
};
use crate::api::request_destination::RequestData;

use super::ports::{BatchCounterLimiter, BatchEnqueuedSource, BatchWindowCache};
use super::records::{
    BatchCounterCode, BatchCounterStatus, BatchEnqueuedLimitExceeded, BatchFileUsage,
    BatchLimitCounterAnswer, BatchReservationWindow,
};
use super::rejections::{self, Attribution, RateLimitRejection};
use super::{BatchLimitCounters, BatchLimitCountersError, CountingFailure};

impl<Limiter, Store, Cache> BatchLimitCounters<Limiter, Store, Cache>
where
    Limiter: BatchCounterLimiter,
    Store: BatchEnqueuedSource,
    Cache: BatchWindowCache,
{
    pub(super) fn attribution(&self) -> Attribution<'_> {
        Attribution {
            resolver: self.resolver.as_ref(),
            router: self.router.as_deref(),
        }
    }

    // _reserve_batch_enqueued_tokens: reserve the batch's tokens, or raise once a scope is over its allowance
    pub(super) async fn reserve_enqueued_tokens(
        &self,
        data: &RequestData,
        batch_usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let outcome = self
            .store
            .reserve(batch_usage.total_tokens, scopes)
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
                self.enqueued_rejection(&reached, data, batch_usage)
            }
        }
    }

    // _raise_enqueued_limit_error itself: the callable as a direct case
    pub(super) fn enqueued_rejection(
        &self,
        over_limit: &BatchEnqueuedLimitExceeded,
        data: &RequestData,
        batch_usage: &BatchFileUsage,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let error = rejections::enqueued_error(over_limit, data, batch_usage, &self.attribution())?;
        Ok(BatchLimitCounterAnswer::RaiseEnqueuedLimitError {
            error: Box::new(error),
        })
    }

    // _raise_rate_limit_error itself: the callable as a direct case
    pub(super) fn rate_rejection(
        &self,
        facts: &RateLimitRejection<'_>,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let error = rejections::rate_limit_error(facts, &self.attribution())?;
        Ok(BatchLimitCounterAnswer::RaiseRateLimitError {
            error: Box::new(error),
        })
    }

    // _check_and_increment_batch_counters: charge every counter, then reject or stash the day's refunds
    pub(super) async fn check_and_increment(
        &self,
        auth: &UserAPIKeyAuth,
        data: &RequestData,
        batch_usage: &BatchFileUsage,
        descriptors: Option<&[BatchLimitDescriptor]>,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let mut charged = match descriptors {
            Some(given) => given.to_vec(),
            None => self
                .limiter
                .base_descriptors(auth, data)
                .map_err(BatchLimitCountersError::Limits)?,
        };
        let mut increments: Vec<BatchDescriptorIncrements> = charged
            .iter()
            .map(|_descriptor| BatchDescriptorIncrements {
                requests: batch_usage.request_count,
                tokens: batch_usage.total_tokens,
            })
            .collect();
        let (project, project_increments) = self
            .limiter
            .project_io_descriptors(auth, &batch_usage.project_usage());
        charged.extend(project);
        increments.extend(project_increments);
        let response = self
            .limiter
            .atomic_check_and_increment_by_n(&charged, &increments)
            .await
            .map_err(BatchLimitCountersError::Limits)?;
        self.stash.set_batch_tpd_refund_ops(Vec::new());
        if response.overall_code == BatchCounterCode::OverLimit {
            let requested_model = rejections::request_model(data);
            for status in &response.statuses {
                if status.code != BatchCounterCode::OverLimit {
                    continue;
                }
                let window_start = self.tpd_window_start(status).await?;
                let facts = RateLimitRejection {
                    status,
                    descriptors: &charged,
                    batch_usage,
                    limit_type: status.rate_limit_type.as_str(),
                    requested_model,
                    window_start,
                    now: (self.clock)(),
                    window_size: self.window_size,
                };
                let error = rejections::rate_limit_error(&facts, &self.attribution())?;
                return Ok(BatchLimitCounterAnswer::CheckAndIncrementBatchCounters {
                    rejection: Some(Box::new(error)),
                });
            }
        }
        let operations = rejections::refund_ops(
            &charged,
            batch_usage.total_tokens,
            &response.reservation_windows,
            &self.limiter,
        )?;
        self.stash.set_batch_tpd_refund_ops(operations);
        Ok(BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection: None })
    }

    // _read_tpd_window_start: only a daily counter has a window key, and a value float cannot hold is absent
    pub(super) async fn read_window_start(
        &self,
        status: &BatchCounterStatus,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let window_start = self.tpd_window_start(status).await?;
        Ok(BatchLimitCounterAnswer::ReadTpdWindowStart { window_start })
    }

    async fn tpd_window_start(
        &self,
        status: &BatchCounterStatus,
    ) -> Result<Option<i64>, CountingFailure<Limiter, Store, Cache>> {
        if !status.descriptor_key.ends_with(TPD_SUFFIX) {
            return Ok(None);
        }
        let value = status.descriptor_value.as_deref().unwrap_or("");
        let key = format!("{{{}:{}}}:window", status.descriptor_key, value);
        let stored = self
            .cache
            .get(&key)
            .await
            .map_err(BatchLimitCountersError::Window)?;
        Ok(stored.and_then(|start| start.seconds()))
    }

    // _build_tpd_refund_ops itself: the callable as a direct case
    pub(super) fn refund_operations(
        &self,
        descriptors: &[BatchLimitDescriptor],
        tokens: i64,
        reservation_windows: &BTreeSet<BatchReservationWindow>,
    ) -> Result<BatchLimitCounterAnswer, CountingFailure<Limiter, Store, Cache>> {
        let operations =
            rejections::refund_ops(descriptors, tokens, reservation_windows, &self.limiter)?;
        Ok(BatchLimitCounterAnswer::BuildTpdRefundOps { operations })
    }
}
