use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::assembled::{
    ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE, is_assembled_stream_success,
};
use super::callback_failure::handle_callback_failure;
use super::delivery::{callback_logger, deliver_async, deliver_sync, global_loggers, logger_event};
use super::payload_state::{set_standard_logging_object, standard_logging_object};
use super::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use super::prepare::{Prepared, prepare};
use super::redaction::redacted_payload;
use super::types::{PASS_THROUGH_CALL_TYPE, SuccessMode, SuccessRequest};
use crate::callback_logger::CallbackLogger;
use crate::dynamic_callbacks::DynamicCallbackLists;
use crate::dynamic_callbacks::filters::get_combined_callback_list;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::LoggingState;
use crate::logging_state::callbacks::{CallbackKind, should_run_callback};
use crate::logging_state::correlation::{CorrelationContext, restore_correlation_context};
use crate::logging_state::events::{
    LoggingEvent, has_run_logging, is_sync_litellm_request, should_run_logging,
};
use crate::registry::CallbackRegistry;

pub struct SuccessHandler<'handler> {
    pub registry: &'handler CallbackRegistry,
    pub payload_builder: &'handler dyn StandardPayloadBuilder,
    pub stream_assembler: &'handler dyn StreamAssembler,
    pub clock: &'handler dyn SuccessClock,
    pub failure_counters: &'handler [Arc<dyn CallbackFailureCounter + Send + Sync>],
    pub excluded_fields: &'handler [String],
    pub known_callback: &'handler (dyn Fn(&str) -> bool + Send + Sync),
    pub stamp: &'handler ResponseStamp,
}

pub fn success_handler(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) {
    success_handler_body(handler, state, request);
    restore_correlation_context(state, correlation);
}

pub async fn async_success_handler(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) {
    async_success_handler_body(handler, state, request).await;
    restore_correlation_context(state, correlation);
}

fn success_handler_body(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    request: &SuccessRequest,
) {
    if !should_run_logging(state, LoggingEvent::SyncSuccess) {
        return;
    }
    let prepared = prepare(handler, state, request);
    let is_sync_request = is_sync_litellm_request(&state.litellm_params);
    if state
        .model_call_details
        .contains_key(COMPLETE_STREAMING_RESPONSE)
    {
        return;
    }
    let complete = complete_stream(handler, state, &prepared, SuccessMode::Sync);
    let lists = DynamicCallbackLists::from(&state.dynamic_callbacks);
    let callbacks =
        get_combined_callback_list(lists.success.as_deref(), &global_loggers(handler.registry));
    has_run_logging(state, LoggingEvent::SyncSuccess);
    for callback in &callbacks {
        let Some(logger) = callback_logger(callback) else {
            continue;
        };
        if !should_run_callback(state, CallbackKind::External, &state.litellm_params)
            || !is_sync_request
            || state.call_type == PASS_THROUGH_CALL_TYPE
        {
            continue;
        }
        deliver_sync_callback(handler, state, &prepared, complete.as_ref(), logger);
    }
}

async fn async_success_handler_body(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    request: &SuccessRequest,
) {
    if !is_assembled_stream_success(state, &request.result)
        && !should_run_logging(state, LoggingEvent::AsyncSuccess)
    {
        return;
    }
    let prepared = prepare(handler, state, request);
    if state
        .model_call_details
        .contains_key(ASYNC_STREAM_RESPONSE_KEY)
    {
        return;
    }
    let complete = complete_stream(handler, state, &prepared, SuccessMode::Async);
    let lists = DynamicCallbackLists::from(&state.dynamic_callbacks);
    let callbacks = get_combined_callback_list(
        lists.async_success.as_deref(),
        &global_loggers(handler.registry),
    );
    has_run_logging(state, LoggingEvent::AsyncSuccess);
    for callback in &callbacks {
        let Some(logger) = callback_logger(callback) else {
            continue;
        };
        if !should_run_callback(state, CallbackKind::External, &state.litellm_params) {
            continue;
        }
        deliver_async_callback(handler, state, &prepared, complete.as_ref(), logger).await;
    }
}

fn deliver_sync_callback(
    handler: &SuccessHandler<'_>,
    state: &LoggingState,
    prepared: &Prepared,
    complete: Option<&Json>,
    logger: &Arc<dyn CallbackLogger>,
) {
    let Some(payload) = event_payload(handler, state, prepared) else {
        return;
    };
    let event = logger_event(state, payload, prepared.guardrail_snapshot.as_ref());
    let stream_event = state.stream == Some(true) && complete.is_none();
    if let Err(error) = deliver_sync(logger.as_ref(), &event, stream_event) {
        handle_callback_failure(handler, logger.callback_name(), &error);
    }
}

async fn deliver_async_callback(
    handler: &SuccessHandler<'_>,
    state: &LoggingState,
    prepared: &Prepared,
    complete: Option<&Json>,
    logger: &Arc<dyn CallbackLogger>,
) {
    let Some(payload) = event_payload(handler, state, prepared) else {
        return;
    };
    let redacted = match redacted_payload(
        &payload,
        logger.as_ref(),
        handler.excluded_fields,
        handler.stamp,
    ) {
        Ok(redacted) => redacted,
        Err(error) => {
            tracing::warn!(
                callback = logger.callback_name(),
                %error,
                "the standard logging payload could not be redacted"
            );
            return;
        }
    };
    let event = logger_event(state, redacted, prepared.guardrail_snapshot.as_ref());
    let stream_event = state.stream == Some(true) && complete.is_none();
    if let Err(error) = deliver_async(logger.as_ref(), event, stream_event).await {
        handle_callback_failure(handler, logger.callback_name(), &error);
    }
}

fn complete_stream(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    prepared: &Prepared,
    mode: SuccessMode,
) -> Option<Json> {
    let complete = handler
        .stream_assembler
        .assemble(state, &prepared.result, mode)?;
    let key = match mode {
        SuccessMode::Sync => COMPLETE_STREAMING_RESPONSE,
        SuccessMode::Async => ASYNC_STREAM_RESPONSE_KEY,
    };
    state
        .model_call_details
        .insert(key.to_owned(), complete.clone());
    let payload =
        handler
            .payload_builder
            .build(state, &complete, prepared.start_time, prepared.end_time);
    if let Err(error) = set_standard_logging_object(state, payload.as_ref()) {
        tracing::warn!(%error, "the assembled stream payload could not be stored");
    }
    Some(complete)
}

// The event's payload is the standard logging object; a chunk without one asks B190 for a fresh build
fn event_payload(
    handler: &SuccessHandler<'_>,
    state: &LoggingState,
    prepared: &Prepared,
) -> Option<StandardLoggingPayload> {
    match standard_logging_object(state) {
        Ok(Some(payload)) => Some(payload),
        Ok(None) => handler.payload_builder.build(
            state,
            prepared.result.response(),
            prepared.start_time,
            prepared.end_time,
        ),
        Err(error) => {
            tracing::warn!(%error, "the standard logging payload could not be read");
            None
        }
    }
}
