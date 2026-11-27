use std::sync::Arc;

use betterlite_models::decisions::Json;
use uuid::Uuid;

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::logging_failure::callbacks::FailureCallbacks;
use crate::logging_failure::handlers::dispatch_failure_handlers;
use crate::logging_failure::{
    BaselineInvalidator, FailureCall, FailureClock, FailureDeps, FailureReport, PostCallFlush,
    RateLimitFailureListener, SyncFailureExecutor,
};
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::assembled::is_assembled_stream_success;
use crate::logging_success::dispatch::dispatch_success_handlers;
use crate::logging_success::handler::SuccessHandler;
use crate::logging_success::payload_state::standard_logging_object;
use crate::logging_success::ports::{StreamAssembler, SuccessClock};
use crate::logging_success::types::SuccessRequest;
use crate::payload::builder::PayloadBuilder;
use crate::payload::emit::PayloadEmitter;
use crate::registry::CallbackRegistry;

pub const FAILED_LOG_MARKER: &str = "lifecycle_failed";
pub const CANCELLED_LOG_MARKER: &str = "lifecycle_cancelled";
pub const TERMINAL_PAYLOAD_MARKER: &str = "lifecycle_terminal_payload";

pub struct LoggingLifecycleConfig {
    pub registry: Arc<CallbackRegistry>,
    pub payload: PayloadBuilder,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub success_clock: Arc<dyn SuccessClock>,
    pub failure_clock: Arc<dyn FailureClock>,
    pub executor: Arc<dyn SyncFailureExecutor>,
    pub baseline: Option<Arc<dyn BaselineInvalidator>>,
    pub flush: Option<Arc<dyn PostCallFlush>>,
    pub special_listeners: Vec<Arc<dyn RateLimitFailureListener>>,
    pub counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub excluded_fields: Vec<String>,
    pub known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub emitter: Arc<dyn PayloadEmitter>,
}

// The success and failure dispatches of one logging track, in the order LiteLLM's Logging runs them.
pub struct LoggingLifecycle {
    registry: Arc<CallbackRegistry>,
    payload: PayloadBuilder,
    stream_assembler: Arc<dyn StreamAssembler>,
    success_clock: Arc<dyn SuccessClock>,
    failure_clock: Arc<dyn FailureClock>,
    executor: Arc<dyn SyncFailureExecutor>,
    baseline: Option<Arc<dyn BaselineInvalidator>>,
    flush: Option<Arc<dyn PostCallFlush>>,
    special_listeners: Vec<Arc<dyn RateLimitFailureListener>>,
    counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    excluded_fields: Vec<String>,
    known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    emitter: Arc<dyn PayloadEmitter>,
}

impl LoggingLifecycle {
    #[must_use]
    pub fn new(config: LoggingLifecycleConfig) -> Self {
        Self {
            registry: config.registry,
            payload: config.payload,
            stream_assembler: config.stream_assembler,
            success_clock: config.success_clock,
            failure_clock: config.failure_clock,
            executor: config.executor,
            baseline: config.baseline,
            flush: config.flush,
            special_listeners: config.special_listeners,
            counters: config.counters,
            excluded_fields: config.excluded_fields,
            known_callback: config.known_callback,
            emitter: config.emitter,
        }
    }

    // A cancelled or failed call never logs a success; a chunk logs its event but no payload.
    pub async fn success(
        &self,
        state: &mut LoggingState,
        correlation: &mut CorrelationContext,
        request: &SuccessRequest,
    ) {
        if logging_stopped(state) {
            return;
        }
        let handler = SuccessHandler {
            registry: self.registry.as_ref(),
            payload_builder: &self.payload,
            stream_assembler: self.stream_assembler.as_ref(),
            clock: self.success_clock.as_ref(),
            excluded_fields: &self.excluded_fields,
            known_callback: self.known_callback.as_ref(),
            privacy: self.payload.privacy,
        };
        dispatch_success_handlers(&handler, state, correlation, request).await;
        let complete =
            state.stream != Some(true) || is_assembled_stream_success(state, &request.result);
        if !complete || !claim_terminal(state) {
            return;
        }
        self.emit(state);
    }

    pub async fn failure(
        &self,
        call: &mut FailureCall<'_>,
        report: &FailureReport,
        callbacks: &FailureCallbacks<'_>,
        prefer_async: bool,
    ) {
        call.state
            .model_call_details
            .insert(FAILED_LOG_MARKER.to_owned(), Json::Bool(true));
        let counters: Vec<&(dyn CallbackFailureCounter + Send + Sync)> =
            self.counters.iter().map(Arc::as_ref).collect();
        // the failure side stamps its redacted response at dispatch, as LiteLLM builds one per redacted payload
        let stamp = ResponseStamp {
            id: format!("chatcmpl-{}", Uuid::new_v4()),
            created: self.failure_clock.now().timestamp(),
        };
        let deps = FailureDeps {
            redactor: self.payload.redactor.as_ref(),
            payloads: &self.payload,
            clock: self.failure_clock.as_ref(),
            executor: self.executor.as_ref(),
            baseline: self.baseline.as_deref(),
            flush: self.flush.as_deref(),
            special_listeners: &self.special_listeners,
            counters,
            excluded_fields: &self.excluded_fields,
            stamp,
            privacy: self.payload.privacy,
        };
        dispatch_failure_handlers(call, report, callbacks, &deps, prefer_async).await;
    }

    pub fn cancel(&self, state: &mut LoggingState) {
        state
            .model_call_details
            .insert(CANCELLED_LOG_MARKER.to_owned(), Json::Bool(true));
    }

    fn emit(&self, state: &LoggingState) {
        match standard_logging_object(state) {
            Ok(Some(payload)) => {
                if let Err(error) = self.emitter.emit(&payload) {
                    tracing::warn!(%error, "the standard logging payload could not be printed");
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "the standard logging payload could not be read to print");
            }
        }
    }
}

fn logging_stopped(state: &LoggingState) -> bool {
    [FAILED_LOG_MARKER, CANCELLED_LOG_MARKER]
        .iter()
        .any(|key| state.model_call_details.get(*key) == Some(&Json::Bool(true)))
}

fn claim_terminal(state: &mut LoggingState) -> bool {
    if state.model_call_details.get(TERMINAL_PAYLOAD_MARKER) == Some(&Json::Bool(true)) {
        return false;
    }
    state
        .model_call_details
        .insert(TERMINAL_PAYLOAD_MARKER.to_owned(), Json::Bool(true));
    true
}
