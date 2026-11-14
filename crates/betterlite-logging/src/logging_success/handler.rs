use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;

use super::assembled::{
    ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE, is_assembled_stream_success,
};
use super::callback_failure::handle_callback_failure;
use super::delivery::{
    callback_logger, deliver_async, deliver_sync, global_loggers, may_run_callback,
};
use super::payload_state::{STANDARD_LOGGING_OBJECT, standard_logging_object};
use super::ports::{StandardPayloadBuilder, StreamAssembler, SuccessClock};
use super::prepare::{Prepared, prepare, store_payload};
use super::redaction::redacted_payload;
use super::request_redaction::{
    redact_payload, redact_stored_payload, should_redact_message_logging,
};
use super::types::{PASS_THROUGH_CALL_TYPE, SuccessMode, SuccessRequest};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::event::LoggerEvent;
use crate::dynamic_callbacks::DynamicCallback;
use crate::dynamic_callbacks::DynamicCallbackLists;
use crate::dynamic_callbacks::filters::get_combined_callback_list;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_metadata::params::metadata_from_litellm_params;
use crate::logging_state::LoggingState;
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
    pub privacy: Settings,
}

struct DeliveryPlan {
    prepared: Prepared,
    callbacks: Vec<DynamicCallback>,
    stream_event: bool,
    redact: bool,
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
    // the async hook loop redacts the shared payload for every logger once one has message_logging off
    let opted_out = mode == SuccessMode::Async
        && callbacks
            .iter()
            .filter_map(callback_logger)
            .any(|logger| !logger.message_logging());
    let redact = opted_out || should_redact_message_logging(state, handler.privacy);
    if redact {
        redact_stored_payload(state);
    }
    has_run_logging(state, logging_event(mode));
    Some(DeliveryPlan {
        stream_event: state.stream == Some(true) && complete.is_none(),
        prepared,
        callbacks,
        redact,
    })
}

fn run_sync_callbacks(handler: &SuccessHandler<'_>, state: &LoggingState, plan: &DeliveryPlan) {
    let is_sync_request = is_sync_litellm_request(&state.litellm_params);
    for logger in plan.callbacks.iter().filter_map(callback_logger) {
        if !may_run_callback(state, logger.as_ref())
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
        if !may_run_callback(logging, logger.as_ref()) {
            continue;
        }
        let Some(event) = callback_event(handler, logging, plan, logger, SuccessMode::Async) else {
            continue;
        };
        if let Err(failure) = deliver_async(logger.as_ref(), event, plan.stream_event).await {
            handle_callback_failure(handler, logger.callback_name(), &failure);
        }
    }
}

fn callback_event(
    handler: &SuccessHandler<'_>,
    state: &LoggingState,
    plan: &DeliveryPlan,
    logger: &Arc<dyn CallbackLogger>,
    mode: SuccessMode,
) -> Option<Arc<LoggerEvent>> {
    let stored = event_payload(handler, state, plan)?;
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
    Some(Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(state.callback_dynamic_params.clone())),
        litellm_metadata: Arc::new(metadata_from_litellm_params(&state.litellm_params)),
        stream: state.stream == Some(true),
        guardrail_snapshot: plan.prepared.guardrail_snapshot.clone(),
    }))
}

fn complete_stream(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    prepared: &Prepared,
    mode: SuccessMode,
) -> Option<Json> {
    let assembled = handler
        .stream_assembler
        .assemble(state, &prepared.result, mode);
    let Some(complete) = assembled else {
        return pass_through_response(handler, state, prepared, mode);
    };
    state
        .model_call_details
        .insert(stream_key(mode).to_owned(), complete.clone());
    store_built_payload(handler, state, prepared, &complete);
    Some(complete)
}

// the async body logs a pass-through result as the complete response; its cost stays unset unless a handler set one
fn pass_through_response(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    prepared: &Prepared,
    mode: SuccessMode,
) -> Option<Json> {
    if mode != SuccessMode::Async || state.call_type != PASS_THROUGH_CALL_TYPE {
        return None;
    }
    let response = prepared.result.response().clone();
    state
        .model_call_details
        .insert(ASYNC_STREAM_RESPONSE_KEY.to_owned(), response.clone());
    state
        .model_call_details
        .entry("response_cost".to_owned())
        .or_insert(Json::Null);
    if !state
        .model_call_details
        .contains_key(STANDARD_LOGGING_OBJECT)
    {
        store_built_payload(handler, state, prepared, &response);
    }
    Some(response)
}

fn store_built_payload(
    handler: &SuccessHandler<'_>,
    state: &mut LoggingState,
    prepared: &Prepared,
    response: &Json,
) {
    let payload =
        handler
            .payload_builder
            .build(state, response, prepared.start_time, prepared.end_time);
    store_payload(state, payload.as_ref());
}

// The event's payload is the standard logging object; a chunk without one asks B190 for a fresh build
fn event_payload(
    handler: &SuccessHandler<'_>,
    state: &LoggingState,
    plan: &DeliveryPlan,
) -> Option<StandardLoggingPayload> {
    match standard_logging_object(state) {
        Ok(Some(payload)) => Some(payload),
        Ok(None) => {
            let mut built = handler.payload_builder.build(
                state,
                plan.prepared.result.response(),
                plan.prepared.start_time,
                plan.prepared.end_time,
            )?;
            if plan.redact {
                redact_payload(&mut built);
            }
            Some(built)
        }
        Err(error) => {
            tracing::warn!(%error, "the standard logging payload could not be read");
            None
        }
    }
}
