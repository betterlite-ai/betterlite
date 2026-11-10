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
use crate::callback_logger::event::LoggerEvent;
use crate::dynamic_callbacks::DynamicCallback;
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

struct DeliveryPlan {
    prepared: Prepared,
    callbacks: Vec<DynamicCallback>,
    stream_event: bool,
}

pub fn success_handler(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    correlation: &mut CorrelationContext,
    request: &SuccessRequest,
) {
    let pending = begin(handler, state, request, SuccessMode::Sync);
    if let Some(plan) = pending {
        run_sync_callbacks(handler, state, &plan);
    }
    // LiteLLM restores the caller's trace and session ids once this call's own logging is done
    restore_correlation_context(state, correlation);
}

pub async fn async_success_handler(
    handler: &SuccessHandler<'_>,
    logging: &mut LoggingState,
    context: &mut CorrelationContext,
    call: &SuccessRequest,
) {
    tracing::debug!(cache_hit = ?call.cache_hit, "LiteLLM-Async Success Call");
    if let Some(plan) = begin(handler, logging, call, SuccessMode::Async) {
        run_async_callbacks(handler, logging, &plan).await;
    }
    restore_correlation_context(logging, context);
}

const fn stream_key(mode: SuccessMode) -> &'static str {
    match mode {
        SuccessMode::Sync => COMPLETE_STREAMING_RESPONSE,
        SuccessMode::Async => ASYNC_STREAM_RESPONSE_KEY,
    }
}

const fn logging_event(mode: SuccessMode) -> LoggingEvent {
    match mode {
        SuccessMode::Sync => LoggingEvent::SyncSuccess,
        SuccessMode::Async => LoggingEvent::AsyncSuccess,
    }
}

// the gate, _success_handler_helper_fn and the assembled-stream guard, then the callbacks to notify
fn begin(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    request: &SuccessRequest,
    mode: SuccessMode,
) -> Option<DeliveryPlan> {
    let assembled =
        mode == SuccessMode::Async && is_assembled_stream_success(state, &request.result);
    if !assembled && !should_run_logging(state, logging_event(mode)) {
        return None;
    }
    let prepared = prepare(handler, state, request);
    if state.model_call_details.contains_key(stream_key(mode)) {
        return None;
    }
    let complete = complete_stream(handler, state, &prepared, mode);
    let lists = DynamicCallbackLists::from(&state.dynamic_callbacks);
    let dynamic = match mode {
        SuccessMode::Sync => lists.success,
        SuccessMode::Async => lists.async_success,
    };
    let callbacks =
        get_combined_callback_list(dynamic.as_deref(), &global_loggers(handler.registry));
    has_run_logging(state, logging_event(mode));
    Some(DeliveryPlan {
        stream_event: state.stream == Some(true) && complete.is_none(),
        prepared,
        callbacks,
    })
}

fn run_sync_callbacks(handler: &SuccessHandler<'_>, state: &LoggingState, plan: &DeliveryPlan) {
    let is_sync_request = is_sync_litellm_request(&state.litellm_params);
    for logger in plan.callbacks.iter().filter_map(callback_logger) {
        if !should_run_callback(state, CallbackKind::External, &state.litellm_params)
            || !is_sync_request
            || state.call_type == PASS_THROUGH_CALL_TYPE
        {
            continue;
        }
        if let Some(event) = callback_event(handler, state, plan, logger, SuccessMode::Sync)
            && let Err(error) = deliver_sync(logger.as_ref(), &event, plan.stream_event)
        {
            handle_callback_failure(handler, logger.callback_name(), &error);
        }
    }
}

async fn run_async_callbacks(
    handler: &SuccessHandler<'_>,
    logging: &LoggingState,
    plan: &DeliveryPlan,
) {
    for logger in plan.callbacks.iter().filter_map(callback_logger) {
        if !should_run_callback(logging, CallbackKind::External, &logging.litellm_params) {
            continue;
        }
        let Some(event) = callback_event(handler, logging, plan, logger, SuccessMode::Async) else {
            continue;
        };
        let delivered = deliver_async(logger.as_ref(), event, plan.stream_event).await;
        if let Err(failure) = delivered {
            handle_callback_failure(handler, logger.callback_name(), &failure);
        }
    }
}

// the async path hands each logger a payload redacted with that logger's own message flags
fn callback_event(
    handler: &SuccessHandler<'_>,
    state: &LoggingState,
    plan: &DeliveryPlan,
    logger: &Arc<dyn CallbackLogger>,
    mode: SuccessMode,
) -> Option<Arc<LoggerEvent>> {
    let stored = event_payload(handler, state, &plan.prepared)?;
    let payload = match mode {
        SuccessMode::Sync => stored,
        SuccessMode::Async => match redacted_payload(
            &stored,
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
                return None;
            }
        },
    };
    Some(logger_event(
        state,
        payload,
        plan.prepared.guardrail_snapshot.as_ref(),
    ))
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
    state
        .model_call_details
        .insert(stream_key(mode).to_owned(), complete.clone());
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
