use std::sync::{Arc, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_core::api::responses_stream_hooks::events::nested_response_mut;
use betterlite_core::api::responses_stream_hooks::ports::{
    CacheWrite, ChunkDeploymentHook, ChunkHookCall, DynamicCacheObject, FailureDelivery,
    PortFailure, ResponseMetadataCall, ResponseMetadataTarget, ResponsesStreamCacheHandler,
    ResponsesStreamCacheTarget, ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
    SuccessDeploymentHook,
};
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::support::Probe;

impl Probe {
    fn record(&self, value: Value) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }
}

impl ResponsesStreamCacheHandler for Probe {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>> {
        self.row.kwargs.as_ref()
    }
    fn preset_cache_key(&self) -> Option<&str> {
        self.row.preset.as_deref()
    }
    fn original_function(&self) -> Option<&str> {
        Some("aresponses")
    }
    fn dual_cache(&self) -> Option<DynamicCacheObject> {
        None
    }
    fn should_store_result_in_cache(
        &self,
        original_function: Option<&str>,
        kwargs: &IndexMap<String, Json>,
    ) -> Result<bool, PortFailure> {
        self.record(json!(["eligible", original_function, kwargs]));
        Ok(self.row.eligible)
    }
}

#[async_trait]
impl ResponsesStreamCacheTarget for Probe {
    fn add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        let payload: Value = serde_json::from_str(&write.payload).map_err(|error| PortFailure {
            message: error.to_string(),
        })?;
        self.record(json!(["cache", write.request_kwargs, payload.get("id")]));
        if self.row.broken {
            return Err(PortFailure {
                message: "broken".to_owned(),
            });
        }
        Ok(())
    }
    async fn async_add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.add_cache(write)
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for Probe {
    async fn dispatch_success_handlers(&self, request: SuccessRequest) {
        self.record(json!([
            "success",
            request.cache_hit,
            request.prefer_async_handlers
        ]));
    }
    async fn async_success_handler(&self, request: &SuccessRequest) {
        self.record(json!(["async_success", request.cache_hit]));
    }
    fn success_handler(&self, request: &SuccessRequest) {
        self.record(json!(["sync_success", request.cache_hit]));
    }
    fn deferred_armed(&self) -> bool {
        self.row.deferred
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for Probe {
    async fn async_failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(json!([
            "async_failure",
            delivery.exception.metadata().status_code
        ]));
        if self.row.broken {
            return Err(PortFailure {
                message: "broken".to_owned(),
            });
        }
        Ok(())
    }
    fn failure_handler(&self, delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(json!([
            "sync_failure",
            delivery.exception.metadata().status_code
        ]));
        Ok(())
    }
}

impl ResponseMetadataTarget for Probe {
    fn update_response_metadata(&self, call: ResponseMetadataCall<'_>) -> Result<(), PortFailure> {
        self.record(json!(["metadata", call.kwargs, call.include_overhead]));
        Ok(())
    }
}

#[async_trait]
impl SuccessDeploymentHook for Probe {
    async fn success_deployment_hook(
        &self,
        request_data: &IndexMap<String, Json>,
        _response: &Json,
        call_type: &str,
    ) -> Result<Json, PortFailure> {
        self.record(json!(["post", request_data, call_type]));
        Ok(Json::Null)
    }
}

pub(super) struct Chunk {
    pub action: String,
    pub probe: Arc<Probe>,
}

#[async_trait]
impl ChunkDeploymentHook for Chunk {
    async fn post_streaming_deployment_hook(
        &self,
        call: ChunkHookCall<'_>,
    ) -> Result<Option<ResponsesAPIStreamingResponse>, PortFailure> {
        self.probe.record(json!([
            "chunk",
            self.action,
            call.call_type,
            call.request_data
        ]));
        match self.action.as_str() {
            "keep" => Ok(None),
            "fail" => Err(PortFailure {
                message: "broken".to_owned(),
            }),
            replacement => {
                let mut event = call.chunk.clone();
                if let Some(response) = nested_response_mut(&mut event) {
                    replacement.clone_into(&mut response.id);
                }
                Ok(Some(event))
            }
        }
    }
}

impl ResponsesCostCalculator for Probe {
    type Error = PortFailure;
    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, PortFailure> {
        self.record(json!(["cost"]));
        Ok(Some(Decimal::ONE))
    }
}
