use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use thiserror::Error;

use crate::routes::CallType;

// LiteLLM's task names, which also prefix the log line a failed task produces
pub const CACHE_WRITE_TASK: &str = "Responses stream cache write";
pub const POST_SUCCESS_TASK: &str = "Responses stream post-call success hook";
pub const FAILURE_LOGGING_TASK: &str = "Responses stream failure logging";
pub const SUCCESS_DISPATCH_TASK: &str = "Responses stream success dispatch";

// A dependency's own failure, carried as text because every caller either logs it or swallows it
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct PortFailure {
    pub message: String,
}

// LiteLLM's dynamic_cache_object: a per-call backend that replaces the configured one, B4297's Dynamic.
// This module forwards it untouched, so it only needs to be shareable and comparable by identity.
pub trait DynamicCacheBackend: Send + Sync {}

impl<T: EnvelopeBackend + ?Sized> DynamicCacheBackend for T {}

#[derive(Clone)]
pub struct DynamicCacheObject(Arc<dyn DynamicCacheBackend>);

impl DynamicCacheObject {
    #[must_use]
    pub fn shared(backend: Arc<dyn DynamicCacheBackend>) -> Self {
        Self(backend)
    }

    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for DynamicCacheObject {
    fn eq(&self, other: &Self) -> bool {
        self.same_as(other)
    }
}

impl Eq for DynamicCacheObject {}

// the caching handler's request kwargs and B4296's store decision over them
pub trait ResponsesStreamCacheHandler: Send + Sync {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>>;

    fn preset_cache_key(&self) -> Option<&str>;

    fn original_function(&self) -> Option<&str>;

    fn dual_cache(&self) -> Option<DynamicCacheObject>;

    fn should_store_result_in_cache(
        &self,
        original_function: Option<&str>,
        kwargs: &IndexMap<String, Json>,
    ) -> Result<bool, PortFailure>;
}

// cache.add_cache(payload, dynamic_cache_object=..., **request_kwargs) and its awaitable twin, B4297's writes
#[async_trait]
pub trait ResponsesStreamCacheTarget: Send + Sync {
    fn add_cache(&self, write: CacheWrite) -> Result<(), PortFailure>;

    async fn async_add_cache(&self, write: CacheWrite) -> Result<(), PortFailure>;
}

pub struct CacheWrite {
    pub payload: String,
    pub request_kwargs: IndexMap<String, Json>,
    pub dynamic_cache: Option<DynamicCacheObject>,
}

// logging_obj's success side: the coroutine dispatch, run_async_function(async_success_handler), executor.submit(success_handler)
#[async_trait]
pub trait ResponsesStreamSuccessTarget: Send + Sync {
    async fn dispatch_success_handlers(&self, request: SuccessRequest);

    async fn async_success_handler(&self, request: &SuccessRequest);

    fn success_handler(&self, request: &SuccessRequest);

    fn deferred_stream_complete_armed(&self) -> bool;
}

// logging_obj's failure side: async_failure_handler awaited, then failure_handler submitted
#[async_trait]
pub trait ResponsesStreamFailureTarget: Send + Sync {
    async fn async_failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure>;

    fn failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure>;
}

// traceback_exception has no Rust source: LiteLLM formats the Python stack of the call site
#[derive(Clone)]
pub struct FailureDelivery {
    pub exception: ErrorClass,
    pub traceback_exception: String,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

// B004's update_response_metadata; the adapter maps include_overhead=false onto the request overhead timing
pub trait ResponseMetadataTarget: Send + Sync {
    fn update_response_metadata(&self, call: ResponseMetadataCall<'_>) -> Result<(), PortFailure>;
}

pub struct ResponseMetadataCall<'call> {
    pub result: &'call ResponsesAPIStreamingResponse,
    pub model: &'call str,
    pub kwargs: &'call IndexMap<String, Json>,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub include_overhead: bool,
}

// one installed async_post_call_streaming_deployment_hook callback, called for every streamed chunk
#[async_trait]
pub trait ChunkDeploymentHook: Send + Sync {
    async fn post_streaming_deployment_hook(
        &self,
        call: ChunkHookCall<'_>,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, PortFailure>;
}

pub struct ChunkHookCall<'call> {
    pub request_data: &'call IndexMap<String, Json>,
    pub chunk: &'call ResponsesAPIStreamingResponse,
    pub call_type: Option<CallType>,
}

// B5367's betterlite_sdk::call_setup::hooks::success_deployment_hook behind the one port this crate can name
#[async_trait]
pub trait SuccessDeploymentHook: Send + Sync {
    async fn success_deployment_hook(
        &self,
        request_data: &IndexMap<String, Json>,
        response: &Json,
        call_type: &str,
    ) -> Result<Json, PortFailure>;
}
