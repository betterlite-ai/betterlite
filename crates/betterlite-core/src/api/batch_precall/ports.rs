use std::error::Error;
use std::future::Future;

use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;

use crate::api::batch_enqueued_refund::store::{EnqueuedLocalStore, ReservationRecordScripts};
use crate::api::batch_enqueued_refund::{
    BatchEnqueuedRefundError, BatchEnqueuedRefundInput, BatchEnqueuedRefundResult,
    EnqueuedTokenRefund,
};
use crate::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use crate::api::batch_enqueued_reserve::BatchEnqueuedTokenScope;
use crate::api::batch_input_accounting::error::BatchHttpError;
use crate::api::batch_limit_counters::records::{BatchFileUsage, BatchTpdRefundOperation};
use crate::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::request_destination::RequestData;

// The hook's four callees: B1044's skip decision, B1046's file accounting, and B1045's reservation or counter charge
pub trait BatchPrecallStages: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn should_skip(
        &self,
        principal: &BatchLimitPrincipal,
        data: &RequestData,
        has_enqueued_scopes: bool,
    ) -> Result<BatchPrecallSkip, Self::Error>;

    fn account(
        &self,
        principal: &BatchLimitPrincipal,
        request: &BatchAccountRequest<'_>,
    ) -> impl Future<Output = Result<BatchFileUsage, Self::Error>> + Send;

    fn reserve(
        &self,
        principal: &BatchLimitPrincipal,
        data: &RequestData,
        usage: &BatchFileUsage,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, Self::Error>> + Send;

    fn charge(
        &self,
        principal: &BatchLimitPrincipal,
        data: &RequestData,
        usage: &BatchFileUsage,
        descriptors: Option<&[BatchLimitDescriptor]>,
    ) -> impl Future<Output = Result<Option<Box<ProxyRateLimitError>>, Self::Error>> + Send;

    // the HTTPException the source re-raises instead of returning the request data
    fn http(&self, error: &Self::Error) -> Option<&BatchHttpError>;
}

#[derive(Debug, Clone)]
pub struct BatchPrecallSkip {
    pub skip: bool,
    pub descriptors: Option<Vec<BatchLimitDescriptor>>,
}

#[derive(Debug, Clone, Copy)]
pub struct BatchAccountRequest<'a> {
    pub file_id: &'a str,
    pub custom_llm_provider: &'a str,
    pub data: &'a RequestData,
    pub descriptors: Option<&'a [BatchLimitDescriptor]>,
}

// The per-request stash B1045 fills: the reservation a retrieve later saves or refunds, and the TPD refunds a failed create applies
pub trait BatchPrecallStash: Send + Sync {
    fn take_reservation(&self) -> Option<BatchEnqueuedTokenReservation>;

    fn take_refund_operations(&self) -> Vec<BatchTpdRefundOperation>;
}

// B1043's refund store, reached through its one public entry so no refund mechanic is duplicated here
pub trait BatchRefundSink: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn run(
        &self,
        input: &BatchEnqueuedRefundInput,
    ) -> impl Future<Output = Result<BatchEnqueuedRefundResult, Self::Error>> + Send;
}

// The token adjustments that apply a batch TPD refund after a failed create
pub trait BatchTokenApplier: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn apply(
        &self,
        operations: &[BatchTpdRefundOperation],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

// FastAPI's HTTPException inside the response-cache hook: the one cache failure the source re-raises
#[derive(Debug, thiserror::Error)]
pub enum BatchCacheError {
    #[error("the batch cache raised an HTTP error with status {status}")]
    Http { status: u16, detail: Json },
    #[error("the batch cache operation failed: {message}")]
    Failed { message: String },
}

// cache.in_memory_cache: the proxy's in-memory tier, read synchronously and written with a ttl
pub trait BatchPrecallMemory: Send + Sync {
    fn keys(&self) -> Vec<String>;

    fn get(&self, key: &str) -> Result<Option<Json>, BatchCacheError>;

    fn set(
        &self,
        key: &str,
        value: Json,
        ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send;

    fn set_many(
        &self,
        entries: Vec<(String, Json)>,
        ttl_seconds: f64,
    ) -> impl Future<Output = Result<(), BatchCacheError>> + Send;
}

// litellm.cache and its backing store: the namespace scan and pipeline fetch on create, the read on retrieve
pub trait BatchPrecallRemote: Send + Sync {
    // litellm.cache.type is not None and litellm.cache.cache is a RedisCache, the two guards the warm-up needs
    fn is_redis(&self) -> bool;

    fn cache_key(&self, data: &RequestData) -> Option<String>;

    fn scan(
        &self,
        pattern: &str,
        count: usize,
    ) -> impl Future<Output = Result<Vec<String>, BatchCacheError>> + Send;

    fn batch_get(
        &self,
        keys: &[String],
    ) -> impl Future<Output = Result<Vec<(String, Json)>, BatchCacheError>> + Send;

    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Json>, BatchCacheError>> + Send;
}

// B1043's store is the refund sink the route wires in; its one entry covers every store method the hooks use
impl<Scripts, Store> BatchRefundSink for EnqueuedTokenRefund<Scripts, Store>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    type Error = BatchEnqueuedRefundError;

    async fn run(
        &self,
        input: &BatchEnqueuedRefundInput,
    ) -> Result<BatchEnqueuedRefundResult, Self::Error> {
        EnqueuedTokenRefund::run(self, input).await
    }
}
