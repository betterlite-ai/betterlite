use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::api::response_synthetic_events::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization,
};
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_core::api::responses_stream_events::{
    ResponsesEventTransform, ResponsesStreamBaseState, ResponsesStreamHiddenParams,
};
use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::ports::{
    CacheWrite, DynamicCacheObject, FailureDelivery, PortFailure, ResponsesStreamCacheHandler,
    ResponsesStreamCacheTarget, ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, transform_streaming_response,
};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::task::TaskTracker;

use super::{CacheMode, CostMode, Input, TransformMode};

pub(crate) struct Probe {
    transform: TransformMode,
    cost: CostMode,
    cache: CacheMode,
    records: Mutex<Vec<Value>>,
    clock: DateTime<FixedOffset>,
    kwargs: IndexMap<String, Json>,
}

impl Probe {
    pub(crate) fn new(input: &Input, clock: DateTime<FixedOffset>) -> Self {
        let mut kwargs = IndexMap::new();
        kwargs.insert("stream".to_owned(), Json::Bool(true));
        Self {
            transform: input.transform,
            cost: input.cost,
            cache: input.cache,
            records: Mutex::new(Vec::new()),
            clock,
            kwargs,
        }
    }

    pub(crate) fn record(&self, value: Value) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }

    pub(crate) fn records(&self) -> Vec<Value> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SuccessClock for Probe {
    fn now(&self) -> DateTime<FixedOffset> {
        self.clock
    }
}

pub(crate) fn hooks<'context>(
    probe: &Arc<Probe>,
    redactor: &'context Redactor,
) -> ResponsesStreamHooks<'context> {
    ResponsesStreamHooks {
        cache: Some(Arc::<Probe>::clone(probe)),
        cache_handler: Some(Arc::<Probe>::clone(probe)),
        success: Some(Arc::<Probe>::clone(probe)),
        failure: Some(Arc::<Probe>::clone(probe)),
        metadata: None,
        success_hook: None,
        chunk_hooks: Vec::new(),
        clock: Arc::<Probe>::clone(probe),
        redactor,
    }
}

impl ResponsesStreamCacheHandler for Probe {
    fn request_kwargs(&self) -> Option<&IndexMap<String, Json>> {
        match self.cache {
            CacheMode::Off => None,
            CacheMode::Failing => Some(&self.kwargs),
        }
    }

    fn preset_cache_key(&self) -> Option<&str> {
        None
    }

    fn original_function(&self) -> Option<&str> {
        Some("aresponses")
    }

    fn dual_cache(&self) -> Option<DynamicCacheObject> {
        None
    }

    fn should_store_result_in_cache(
        &self,
        _original_function: Option<&str>,
        _kwargs: &IndexMap<String, Json>,
    ) -> Result<bool, PortFailure> {
        Ok(true)
    }
}

#[async_trait]
impl ResponsesStreamCacheTarget for Probe {
    fn add_cache(&self, _write: CacheWrite) -> Result<(), PortFailure> {
        Err(PortFailure {
            message: "cache write failed".to_owned(),
        })
    }

    async fn async_add_cache(&self, write: CacheWrite) -> Result<(), PortFailure> {
        self.add_cache(write)
    }
}

#[async_trait]
impl ResponsesStreamSuccessTarget for Probe {
    async fn dispatch_success_handlers(&self, _request: SuccessRequest) {
        self.record(Value::String("success_dispatch".to_owned()));
    }

    async fn async_success_handler(&self, _request: &SuccessRequest) {
        self.record(Value::String("success_async".to_owned()));
    }

    fn success_handler(&self, _request: &SuccessRequest) {
        self.record(Value::String("success_sync".to_owned()));
    }

    fn deferred_armed(&self) -> bool {
        false
    }
}

#[async_trait]
impl ResponsesStreamFailureTarget for Probe {
    async fn async_failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(Value::String("failure_async".to_owned()));
        Ok(())
    }

    fn failure_handler(&self, _delivery: &FailureDelivery) -> Result<(), PortFailure> {
        self.record(Value::String("failure_sync".to_owned()));
        Ok(())
    }
}

pub(crate) struct Calculator {
    probe: Arc<Probe>,
}

impl Calculator {
    pub(crate) fn new(probe: Arc<Probe>) -> Self {
        Self { probe }
    }
}

impl ResponsesCostCalculator for Calculator {
    type Error = CostFailure;

    fn response_cost(&self, target: &CostTarget<'_>) -> Result<Option<Decimal>, CostFailure> {
        let id = match target {
            CostTarget::Model(response) => Json::Text(response.id.clone()),
            CostTarget::Object(fields) => fields.get("id").cloned().unwrap_or(Json::Null),
        };
        self.probe.record(json!(["cost", id]));
        match self.probe.cost {
            CostMode::Positive => Ok(Some(Decimal::new(5, 1))),
            CostMode::Zero => Ok(Some(Decimal::ZERO)),
            CostMode::Missing => Ok(None),
            CostMode::Error => Err(CostFailure),
        }
    }
}

#[derive(Debug, Error)]
#[error("cost calculation failed")]
pub(crate) struct CostFailure;

pub(crate) struct ProviderTransform {
    probe: Arc<Probe>,
}

impl ProviderTransform {
    pub(crate) fn new(probe: Arc<Probe>) -> Self {
        Self { probe }
    }
}

impl ResponsesEventTransform for ProviderTransform {
    type Error = TransformFailure;

    fn transform(
        &self,
        _model: &str,
        chunk: &Json,
        _logging: &mut IndexMap<String, Json>,
    ) -> Result<ResponsesAPIStreamingResponse, TransformFailure> {
        if matches!(self.probe.transform, TransformMode::Failing) {
            return Err(TransformFailure);
        }
        let Json::Object(fields) = chunk else {
            return Err(TransformFailure);
        };
        Ok(event(fields))
    }
}

#[derive(Debug, Error)]
#[error("the provider transform failed")]
pub(crate) struct TransformFailure;

fn event(fields: &IndexMap<String, Json>) -> ResponsesAPIStreamingResponse {
    match transform_streaming_response(fields) {
        DecodedStreamResponse::Known { record, .. } => ResponsesAPIStreamingResponse::Known(record),
        DecodedStreamResponse::Generic(generic) => ResponsesAPIStreamingResponse::Generic(generic),
        DecodedStreamResponse::Lax { fields, .. } => {
            ResponsesAPIStreamingResponse::Generic(generic_event(fields))
        }
    }
}

fn generic_event(fields: IndexMap<String, Json>) -> GenericEvent {
    let r#type = match fields.get("type") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    };
    let extra = fields
        .into_iter()
        .filter(|(name, _value)| name != "type")
        .collect();
    GenericEvent { r#type, extra }
}

pub(crate) struct FixedTokens {
    probe: Arc<Probe>,
}

impl FixedTokens {
    pub(crate) fn new(probe: Arc<Probe>) -> Self {
        Self { probe }
    }
}

impl UsageTokenization for FixedTokens {
    type Error = Infallible;

    fn messages(&self, request: &EstimateRequest<'_>) -> Result<Vec<Json>, Infallible> {
        self.probe.record(json!([
            "estimate",
            request.model,
            request.request_input,
            request.responses_api_request,
            request.generated_text,
        ]));
        Ok(vec![request.request_input.clone()])
    }

    fn count(&self, request: TokenCountRequest<'_>) -> Result<i64, Infallible> {
        if request.count_response_tokens {
            return Ok(as_i64(request.text.map_or(0, str::len)));
        }
        Ok(1000 + as_i64(request.model.len()))
    }
}

fn as_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn state(input: &Input, probe: &Probe) -> ResponsesStreamBaseState {
    ResponsesStreamBaseState {
        model: input.model.clone(),
        custom_llm_provider: input.provider.clone(),
        litellm_metadata: Some(input.metadata.clone()),
        request_data: input.request_data.clone(),
        call_type: Some("responses".to_owned()),
        finished: false,
        failure_handled: false,
        yielded_first_chunk: false,
        output_started: false,
        completed_response: None,
        completed_response_cached: false,
        completed_response_logged: false,
        completed_response_cache_hit: None,
        persist_before_logging: true,
        generated_content: String::new(),
        generated_tool_arguments: String::new(),
        start_time: probe.clock,
        created_timestamp: 0.0,
        max_streaming_duration_seconds: None,
        hidden_params: ResponsesStreamHiddenParams {
            model_id: model_id(&input.metadata),
            api_base: None,
            custom_llm_provider: input.provider.clone(),
            additional_headers: IndexMap::new(),
        },
        raw_response_headers: IndexMap::new(),
        pending_tasks: TaskTracker::new(),
    }
}

fn model_id(metadata: &IndexMap<String, Json>) -> Option<String> {
    let Some(Json::Object(info)) = metadata.get("model_info") else {
        return None;
    };
    match info.get("id") {
        Some(Json::Text(id)) => Some(id.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) fn logging_object(
    redactor: &Redactor,
) -> Result<StreamLoggingObject, LoggingStateError> {
    let mut context = CorrelationContext::default();
    let logging = LoggingState::new(
        LoggingInit {
            model: "gpt-4o".to_owned(),
            call_type: "responses".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(true),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )?;
    Ok(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: context,
        })),
        dispatch: Arc::new(NoDispatch),
    })
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

#[derive(Debug, Error)]
pub(crate) enum LoggingStateError {
    #[error(transparent)]
    Time(#[from] chrono::ParseError),
    #[error(transparent)]
    State(#[from] betterlite_logging::logging_state::LoggingStateError),
}
