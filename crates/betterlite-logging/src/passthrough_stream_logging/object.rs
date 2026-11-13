use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use chrono::{DateTime, FixedOffset};

use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;
use crate::logging_success::dispatch::dispatch_success_handlers;
use crate::logging_success::handler::SuccessHandler;
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::logging_success::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use crate::logging_success::types::{SuccessRequest, SuccessResult};
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::registry::CallbackRegistry;

pub struct StreamLoggingState {
    pub logging: LoggingState,
    pub correlation: CorrelationContext,
}

// The per-request logging object: leaf handlers write the call log through the slot, then the success dispatch owns it.
pub struct StreamLoggingObject {
    pub slot: Mutex<Option<StreamLoggingState>>,
    pub dispatch: Arc<dyn StreamSuccessDispatch>,
}

impl StreamLoggingObject {
    // Empty only while the success dispatch owns the state, which no leaf handler runs under.
    pub fn edit(&self, edit: &mut dyn FnMut(&mut LoggingState)) {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(state) = slot.as_mut() {
            edit(&mut state.logging);
        }
    }

    pub fn read<R>(&self, read: &mut dyn FnMut(&LoggingState) -> R) -> Option<R> {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|state| read(&state.logging))
    }
}

pub struct StreamSuccessCall<'call> {
    pub state: &'call mut LoggingState,
    pub correlation: &'call mut CorrelationContext,
    pub result: SuccessResult,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: &'call PassThroughLoggingKwargs,
}

// The dispatch half of B3605's log dispatcher; B250's handler needs owned references, so this seam carries them per call.
#[async_trait]
pub trait StreamSuccessDispatch: Send + Sync {
    async fn dispatch_success(&self, call: StreamSuccessCall<'_>);
}

pub struct StreamLoggingCallbacks {
    pub registry: Arc<CallbackRegistry>,
    pub payload_builder: Arc<dyn StandardPayloadBuilder>,
    pub stream_assembler: Arc<dyn StreamAssembler>,
    pub clock: Arc<dyn SuccessClock>,
    pub failure_counters: Vec<Arc<dyn CallbackFailureCounter + Send + Sync>>,
    pub excluded_fields: Vec<String>,
    pub known_callback: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub stamp: ResponseStamp,
    pub privacy: Settings,
}

pub struct SuccessHandlerDispatch {
    pub callbacks: StreamLoggingCallbacks,
}

#[async_trait]
impl StreamSuccessDispatch for SuccessHandlerDispatch {
    async fn dispatch_success(&self, call: StreamSuccessCall<'_>) {
        let StreamSuccessCall {
            state,
            correlation,
            result,
            start_time,
            end_time,
            cache_hit,
            kwargs,
        } = call;
        let request = SuccessRequest {
            result,
            start_time: Some(start_time),
            end_time: Some(end_time),
            cache_hit: Some(cache_hit),
            prefer_async_handlers: true,
            standard_logging_object: standard_logging_object_kwarg(kwargs),
            guardrail_snapshot: None,
        };
        let handler = SuccessHandler {
            registry: self.callbacks.registry.as_ref(),
            payload_builder: self.callbacks.payload_builder.as_ref(),
            stream_assembler: self.callbacks.stream_assembler.as_ref(),
            clock: self.callbacks.clock.as_ref(),
            failure_counters: &self.callbacks.failure_counters,
            excluded_fields: &self.callbacks.excluded_fields,
            known_callback: self.callbacks.known_callback.as_ref(),
            stamp: &self.callbacks.stamp,
            privacy: self.callbacks.privacy,
        };
        dispatch_success_handlers(&handler, state, correlation, &request).await;
    }
}

fn standard_logging_object_kwarg(
    kwargs: &PassThroughLoggingKwargs,
) -> Option<StandardLoggingPayload> {
    let value = kwargs.entries.get(STANDARD_LOGGING_OBJECT)?;
    let text = serde_json::to_string(value).ok()?;
    match serde_json::from_str(&text) {
        Ok(payload) => Some(payload),
        Err(error) => {
            tracing::warn!(%error, "the pass-through standard logging object could not be read");
            None
        }
    }
}
