use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::NewRelicAgentError;
use super::events::{ChatMessageEvent, MESSAGE_EVENT_NAME, SUMMARY_EVENT_NAME, SummaryEvent};
use super::logger::NewRelicLogger;
use super::metrics::check_and_emit_periodic_metric;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::timestamps::Moment;
use crate::newrelic_extraction::content::ContentPolicy;
use crate::newrelic_extraction::messages::{MessageEvent, all_messages};
use crate::newrelic_extraction::request_params::from_sources;
use crate::newrelic_extraction::response_fields::{
    completion_id, finish_reason, model_names, usage, vendor,
};
use crate::newrelic_extraction::sources::{CallKwargs, EventSources};
use crate::newrelic_extraction::timing::duration;
use crate::newrelic_extraction::trace_context::resolve_trace_id;
use crate::spend_bodies::redaction::should_redact_message_logging;

pub struct SuccessCall<'event> {
    pub sources: EventSources<'event>,
    pub start_time: Option<Moment>,
    pub end_time: Option<Moment>,
    pub redact_message_logging: bool,
}

pub(super) fn process_event(
    logger: &NewRelicLogger,
    event: &LoggerEvent,
) -> Result<(), NewRelicAgentError> {
    let payload = event.payload.as_ref();
    let empty_response = IndexMap::new();
    let response = match payload.response.as_ref() {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => &empty_response,
    };
    let kwargs = CallKwargs {
        messages: payload.messages.as_ref(),
        model: Some(payload.model.as_str()),
        litellm_call_id: payload.litellm_call_id.as_deref(),
        custom_llm_provider: payload.custom_llm_provider.as_deref(),
        headers: None,
        optional_params: Some(&payload.model_parameters),
        llm_api_duration_ms: None,
        start_time: Some(Moment::Float(payload.start_time)),
        end_time: Some(Moment::Float(payload.end_time)),
    };
    let sources = EventSources {
        kwargs: &kwargs,
        response,
        standard_logging_object: Some(payload),
    };
    // a LoggerEvent has no inbound headers; B1104 derives the traceparent trace id into the payload trace id read here
    let redact_message_logging = match &*event.dynamic_params {
        Json::Object(fields) => should_redact_message_logging(None, Some(fields), false),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    };
    let call = SuccessCall {
        sources,
        start_time: None,
        end_time: None,
        redact_message_logging,
    };
    process_success(logger, &call)
}

pub(super) fn process_success(
    logger: &NewRelicLogger,
    call: &SuccessCall<'_>,
) -> Result<(), NewRelicAgentError> {
    if !logger.enabled {
        return Ok(());
    }
    check_and_emit_periodic_metric(logger);
    let sources = &call.sources;
    let trace_id = resolve_trace_id(sources, logger.ids.as_ref());
    let request_id = logger.ids.next_id().to_string();
    let llm_response_id = completion_id(sources, logger.ids.as_ref());
    let vendor = vendor(sources);
    let (request_model, response_model) = model_names(sources);
    let usage = usage(sources);
    let finish_reason = finish_reason(sources);
    let elapsed = duration(sources, call.start_time.as_ref(), call.end_time.as_ref())
        .map_err(NewRelicAgentError::Extraction)?;
    let request_parameters = from_sources(sources);
    let policy = ContentPolicy {
        turn_off_message_logging: logger.settings.turn_off_message_logging,
        redact_message_logging: call.redact_message_logging,
        environment: logger.environment.as_ref(),
    };
    let messages = all_messages(sources, &response_model, &vendor, &policy)
        .map_err(NewRelicAgentError::Extraction)?;
    let summary = SummaryEvent {
        request_id: request_id.clone(),
        trace_id: trace_id.clone(),
        request_model,
        response_model,
        finish_reason,
        number_of_messages: messages.len(),
        vendor,
        usage,
        duration: elapsed,
        request_parameters,
    };
    record_summary_event(logger, &summary);
    record_message_events(logger, &request_id, &llm_response_id, &trace_id, &messages);
    Ok(())
}

fn record_summary_event(logger: &NewRelicLogger, event: &SummaryEvent) {
    let resolved = logger.agent.as_ref().and_then(|agent| agent.application());
    let Some(application) = resolved.filter(|candidate| candidate.enabled()) else {
        tracing::warn!("New Relic application is not enabled; skipping summary event recording.");
        return;
    };
    if let Err(error) = application.record_custom_event(SUMMARY_EVENT_NAME, &event.attributes()) {
        tracing::warn!(%error, "Failed to record New Relic summary event");
        logger.count_callback_failure();
    }
}

fn record_message_events(
    logger: &NewRelicLogger,
    request_id: &str,
    llm_response_id: &str,
    trace_id: &str,
    messages: &[MessageEvent],
) {
    let resolved = logger.agent.as_ref().and_then(|agent| agent.application());
    let Some(application) = resolved.filter(|candidate| candidate.enabled()) else {
        tracing::warn!("New Relic application is not enabled; skipping message event recording.");
        return;
    };
    for message in messages {
        let event = ChatMessageEvent {
            id: format!("{llm_response_id}-{}", message.sequence),
            request_id: request_id.to_owned(),
            trace_id: trace_id.to_owned(),
            role: message.role.clone(),
            sequence: message.sequence,
            response_model: message.response_model.clone(),
            vendor: message.vendor.clone(),
            content: message.content.clone(),
            is_response: message.is_response.unwrap_or(false),
            timestamp: message.timestamp,
        };
        if let Err(error) = application.record_custom_event(MESSAGE_EVENT_NAME, &event.attributes())
        {
            tracing::warn!(%error, "Failed to record New Relic message events");
            logger.count_callback_failure();
            return;
        }
    }
}
