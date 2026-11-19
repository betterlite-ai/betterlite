use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_core::api::responses_stream_events::{ResponsesApiBase, ResponsesEventTransform};
use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::ports::{
    ChunkDeploymentHook, ChunkHookCall, FailureDelivery, PortFailure, ResponsesStreamFailureTarget,
    ResponsesStreamSuccessTarget,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::api_base_lookup::{ApiBaseError, ApiBaseParams};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::sdk_http::error::SdkHttpError;

pub(super) type Event = ResponsesAPIStreamingResponse;

#[derive(Default)]
pub(super) struct Recorder {
    hooks: Mutex<Vec<String>>,
    logging: Mutex<Vec<String>>,
}

impl Recorder {
    fn push(calls: &Mutex<Vec<String>>, label: &str) {
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(label.to_owned());
    }

    pub(super) fn take(&self) -> (Vec<String>, Vec<String>) {
        let mut hooks =
            std::mem::take(&mut *self.hooks.lock().unwrap_or_else(PoisonError::into_inner));
        let mut logging =
            std::mem::take(&mut *self.logging.lock().unwrap_or_else(PoisonError::into_inner));
        hooks.sort_unstable();
        logging.sort_unstable();
        (hooks, logging)
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for Recorder {
    async fn dispatch_success_handlers(&self, _request: SuccessRequest) {
        Self::push(&self.logging, "success");
    }

    async fn async_success_handler(&self, _request: &SuccessRequest) {
        Self::push(&self.logging, "success");
    }

    fn success_handler(&self, _request: &SuccessRequest) {}

    fn deferred_armed(&self) -> bool {
        false
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for Recorder {
    async fn async_failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        Self::push(&self.logging, "failure");
        Ok(())
    }

    fn failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        Ok(())
    }
}

// LiteLLM runs the callbacks' streaming deployment hook once per yielded event, in order
#[async_trait]
impl ChunkDeploymentHook for Recorder {
    async fn post_streaming_deployment_hook(
        &self,
        _call: ChunkHookCall<'_>,
    ) -> Result<Option<Event>, PortFailure> {
        Self::push(&self.hooks, "chunk");
        Ok(None)
    }
}

pub(super) struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(super) struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::UNIX_EPOCH.fixed_offset()
    }
}

pub(super) struct NoApiBase;

impl ResponsesApiBase for NoApiBase {
    fn resolve(
        &self,
        _model: &str,
        _params: &ApiBaseParams,
    ) -> Result<Option<String>, ApiBaseError> {
        Ok(None)
    }
}

#[derive(Debug, Error)]
#[error("the provider event does not decode")]
pub(super) struct DecodeError(#[source] serde_json::Error);

// B509's provider transform decodes the OpenAI wire event; the driver under test only needs a decode
pub(super) struct Decoder;

impl ResponsesEventTransform for Decoder {
    type Error = DecodeError;

    fn transform(
        &self,
        _model: &str,
        chunk: &Json,
        _logging: &mut IndexMap<String, Json>,
    ) -> Result<Event, Self::Error> {
        let value = serde_json::to_value(chunk).map_err(DecodeError)?;
        serde_json::from_value(value).map_err(DecodeError)
    }
}

pub(super) struct NoTokenization;

impl UsageTokenization for NoTokenization {
    type Error = std::io::Error;

    fn messages(&self, _request: &EstimateRequest<'_>) -> Result<Vec<Json>, Self::Error> {
        Ok(Vec::new())
    }

    fn count(&self, _request: TokenCountRequest<'_>) -> Result<i64, Self::Error> {
        Ok(0)
    }
}

pub(super) struct NoCost;

impl ResponsesCostCalculator for NoCost {
    type Error = std::io::Error;

    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        Ok(None)
    }
}

// Everything one run's hooks report, rebuilt per run so no state leaks between drivers
pub(super) struct Environment {
    pub(super) recorder: Arc<Recorder>,
    redactor: Redactor,
    pub(super) logging: StreamLoggingObject,
    pub(super) limits: ApiLimits,
}

impl Environment {
    pub(super) fn new() -> Result<Self, SdkHttpError> {
        Ok(Self {
            recorder: Arc::new(Recorder::default()),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
            logging: StreamLoggingObject {
                slot: Mutex::new(None),
                dispatch: Arc::new(NoDispatch),
            },
            limits: ApiLimits::default(),
        })
    }

    pub(super) fn hooks(&self) -> ResponsesStreamHooks<'_> {
        let success: Arc<dyn ResponsesStreamSuccessTarget> = Arc::clone(&self.recorder);
        let failure: Arc<dyn ResponsesStreamFailureTarget> = Arc::clone(&self.recorder);
        let chunk: Arc<dyn ChunkDeploymentHook> = Arc::clone(&self.recorder);
        let clock: Arc<dyn SuccessClock> = Arc::new(FixedClock);
        ResponsesStreamHooks {
            cache: None,
            cache_handler: None,
            success: Some(success),
            failure: Some(failure),
            metadata: None,
            success_hook: None,
            chunk_hooks: vec![chunk],
            clock,
            redactor: &self.redactor,
        }
    }
}
