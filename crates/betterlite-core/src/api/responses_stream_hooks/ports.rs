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

#[derive(Clone, Copy)]
pub enum Dispatch {
    Inline,
    Spawned,
}

pub const CACHE_WRITE_TASK: &str = "Responses stream cache write";
pub const POST_SUCCESS_TASK: &str = "Responses stream post-call success hook";
pub const FAILURE_LOGGING_TASK: &str = "Responses stream failure logging";
pub const SUCCESS_DISPATCH_TASK: &str = "Responses stream success dispatch";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct PortFailure {
    pub message: String,
}

pub trait DynamicCacheBackend: Send + Sync {}

impl<T: EnvelopeBackend + ?Sized> DynamicCacheBackend for T {}

#[derive(Clone)]
pub struct DynamicCacheObject(pub Arc<dyn DynamicCacheBackend>);

impl PartialEq for DynamicCacheObject {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for DynamicCacheObject {}

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

#[async_trait]
pub trait ResponsesStreamSuccessTarget: Send + Sync {
    async fn dispatch_success_handlers(&self, request: SuccessRequest);

    async fn async_success_handler(&self, request: &SuccessRequest);

    fn success_handler(&self, request: &SuccessRequest);

    fn deferred_armed(&self) -> bool;
}

#[async_trait]
pub trait ResponsesStreamFailureTarget: Send + Sync {
    async fn async_failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure>;

    fn failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure>;
}

#[derive(Clone)]
pub struct FailureDelivery {
    pub exception: ErrorClass,
    pub traceback_exception: String,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

pub trait ResponseMetadataTarget: Send + Sync {
    fn update_response_metadata(&self, call: ResponseMetadataCall<'_>) -> Result<(), PortFailure>;
}

pub struct ResponseMetadataCall<'call> {
    pub result: &'call mut ResponsesAPIStreamingResponse,
    pub model: &'call str,
    pub kwargs: &'call IndexMap<String, Json>,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub include_overhead: bool,
}

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

#[async_trait]
pub trait SuccessDeploymentHook: Send + Sync {
    async fn success_deployment_hook(
        &self,
        request_data: &IndexMap<String, Json>,
        response: &Json,
        call_type: &str,
    ) -> Result<Json, PortFailure>;
}
