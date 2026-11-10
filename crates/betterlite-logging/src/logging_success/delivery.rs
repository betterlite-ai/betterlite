use std::sync::Arc;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::dynamic_callbacks::DynamicCallback;
use crate::logging_state::LoggingState;
use crate::logging_state::callbacks::{CallbackKind, should_run_callback};
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackPhase;

pub(super) fn global_loggers(registry: &CallbackRegistry) -> Vec<DynamicCallback> {
    registry
        .loggers(CallbackPhase::Success)
        .into_iter()
        .map(DynamicCallback::Logger)
        .collect()
}

const PROXY_CALLBACK_MARKER: &str = "_PROXY_";

pub(super) fn may_run_callback(state: &LoggingState, logger: &dyn CallbackLogger) -> bool {
    let kind = if logger.callback_name().contains(PROXY_CALLBACK_MARKER) {
        CallbackKind::ProxyInternal
    } else {
        CallbackKind::External
    };
    should_run_callback(state, kind, &state.litellm_params)
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
