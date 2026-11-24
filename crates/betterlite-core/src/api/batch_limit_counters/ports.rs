use std::error::Error;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use indexmap::IndexMap;

use crate::api::batch_enqueued_reserve::store::{
    BatchEnqueuedReserveError, EnqueuedTokenCounters, ReservationScripts,
};
use crate::api::batch_enqueued_reserve::{
    BatchEnqueuedReserveResult, BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope,
    BatchEnqueuedTokenStore,
};
use crate::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use crate::api::batch_limit_descriptors::input::BatchModelUsage;
use crate::api::request_destination::RequestData;

use super::records::{
    BatchCounterLimitType, BatchCounterResponse, BatchTpdRefundOperation, WindowStartValue,
};

// The limiter's enqueued-token store; B1042's store is the production implementation
pub trait BatchEnqueuedSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn reserve(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<BatchEnqueuedReserveResult, Self::Error>> + Send;
}

impl<Scripts, Counters> BatchEnqueuedSource for BatchEnqueuedTokenStore<Scripts, Counters>
where
    Scripts: ReservationScripts,
    Counters: EnqueuedTokenCounters,
{
    type Error = BatchEnqueuedReserveError;

    fn reserve(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<BatchEnqueuedReserveResult, Self::Error>> + Send {
        self.reserve_scopes(tokens, scopes)
    }
}

// The v3 limiter surface these callables use: B1044's descriptor builders, the atomic counter and its key format
pub trait BatchCounterLimiter: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn base_descriptors(
        &self,
        auth: &UserAPIKeyAuth,
        data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, Self::Error>;

    fn project_io_descriptors(
        &self,
        auth: &UserAPIKeyAuth,
        per_model_usage: &IndexMap<String, BatchModelUsage>,
    ) -> (Vec<BatchLimitDescriptor>, Vec<BatchDescriptorIncrements>);

    // ast-grep-ignore: no-long-names
    fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[BatchLimitDescriptor],
        increments: &[BatchDescriptorIncrements],
    ) -> impl Future<Output = Result<BatchCounterResponse, Self::Error>> + Send;

    #[must_use]
    fn create_rate_limit_keys(
        &self,
        key: &str,
        value: &str,
        rate_limit_type: BatchCounterLimitType,
    ) -> String;
}

// InternalUsageCache.async_get_cache for a "{key:value}:window" key; a validated int, float or str comes back
pub trait BatchWindowCache: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<WindowStartValue>, Self::Error>> + Send;
}

// RequestRateLimiterStash's two batch fields
pub trait BatchCounterStash: Send + Sync {
    fn set_batch_enqueued_reservation(&self, reservation: BatchEnqueuedTokenReservation);

    fn set_batch_tpd_refund_ops(&self, operations: Vec<BatchTpdRefundOperation>);
}
