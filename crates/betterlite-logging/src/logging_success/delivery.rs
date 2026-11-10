use std::sync::Arc;

use betterlite_models::decisions::Json;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::{GuardrailRequestSnapshot, LoggerEvent};
use crate::dynamic_callbacks::DynamicCallback;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackPhase;

pub(super) fn global_loggers(registry: &CallbackRegistry) -> Vec<DynamicCallback> {
    registry
        .loggers(CallbackPhase::Success)
        .into_iter()
        .map(DynamicCallback::Logger)
        .collect()
}

pub(super) fn callback_logger(callback: &DynamicCallback) -> Option<&Arc<dyn CallbackLogger>> {
    match callback {
        DynamicCallback::Logger(logger) => Some(logger),
        DynamicCallback::Name(name) => {
            tracing::warn!(
                callback = %name,
                "no native logger answers to this callback name; skipping its success event"
            );
            None
        }
    }
}

pub(super) fn deliver_sync(
    logger: &dyn CallbackLogger,
    event: &LoggerEvent,
    stream_event: bool,
) -> Result<(), LoggerError> {
    if stream_event {
        return logger.log_stream_event(event);
    }
    logger.log_success_event(event)
}

pub(super) async fn deliver_async(
    logger: &dyn CallbackLogger,
    event: Arc<LoggerEvent>,
    stream_event: bool,
) -> Result<(), LoggerError> {
    if !stream_event {
        return logger.async_log_success_event(event).await;
    }
    logger.async_log_stream_event(event).await
}

pub(super) fn logger_event(
    state: &LoggingState,
    payload: StandardLoggingPayload,
    guardrail_snapshot: Option<&Arc<GuardrailRequestSnapshot>>,
) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(state.callback_dynamic_params.clone())),
        stream: state.stream == Some(true),
        guardrail_snapshot: guardrail_snapshot.cloned(),
    })
}
