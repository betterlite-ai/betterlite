use std::sync::Arc;

use betterlite_logging::logging_success::types::{SuccessRequest, SuccessResult};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use indexmap::IndexMap;

use super::ResponsesStreamHooks;
use super::cache::value_is_set;
use super::events::{nested_response, nested_response_mut};
use super::ports::{PortFailure, SUCCESS_DISPATCH_TASK};
use super::state::ResponsesStreamHookState;
use crate::api::response_synthetic_events::copies::{LoggedCopy, LoggingEvent, logging_copy};
use crate::api::responses_stream_primitives::state::{
    ResponsesStreamBaseState, ResponsesStreamHiddenParams,
};
use crate::api::responses_stream_primitives::tasks::spawn_logging_task;

#[must_use]
pub fn logging_event(
    event: &ResponsesAPIStreamingResponse,
    hidden: &ResponsesStreamHiddenParams,
    raw_response_headers: &IndexMap<String, String>,
) -> Json {
    let source_hidden = source_hidden_params(event);
    match logging_copy(LoggingEvent::Event(event)) {
        LoggedCopy::Same(value) => value.clone(),
        LoggedCopy::Detached(mut copy) => {
            if let Some(response) = nested_response_mut(copy.as_mut()) {
                response.extra.remove("_hidden_params");
            }
            restore_headers(copy.as_mut(), hidden, raw_response_headers, source_hidden);
            event_json(copy.as_ref())
        }
    }
}

pub fn restore_headers(
    event: &mut ResponsesAPIStreamingResponse,
    hidden: &ResponsesStreamHiddenParams,
    raw_response_headers: &IndexMap<String, String>,
    source_hidden: Option<&IndexMap<String, Json>>,
) {
    let Some(response) = nested_response_mut(event) else {
        return;
    };
    let existing = match response.extra.get("_hidden_params") {
        Some(Json::Object(fields)) => Some(fields.clone()),
        _ => None,
    };
    let processed = source_hidden.and_then(|fields| fields.get("additional_headers"));
    let raw = source_hidden.and_then(|fields| fields.get("headers"));
    let headers = copied_header_map(processed, &hidden.additional_headers);
    let raw_headers = copied_header_map(raw, &raw_headers_map(raw_response_headers));
    let mut restored = IndexMap::new();
    restored.insert("additional_headers".to_owned(), Json::Object(headers));
    restored.insert("headers".to_owned(), Json::Object(raw_headers));
    if let Some(existing) = existing {
        restored.extend(existing);
    }
    response
        .extra
        .insert("_hidden_params".to_owned(), Json::Object(restored));
}

fn copied_header_map(
    candidate: Option<&Json>,
    fallback: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let Some(value) = candidate.filter(|value| value_is_set(Some(value))) else {
        return fallback.clone();
    };
    match value {
        Json::Object(fields) => fields.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    }
}

fn raw_headers_map(raw_response_headers: &IndexMap<String, String>) -> IndexMap<String, Json> {
    raw_response_headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

fn source_hidden_params(event: &ResponsesAPIStreamingResponse) -> Option<&IndexMap<String, Json>> {
    match nested_response(event)?.extra.get("_hidden_params") {
        Some(Json::Object(fields)) => Some(fields),
        _ => None,
    }
}

fn event_json(event: &ResponsesAPIStreamingResponse) -> Json {
    serde_json::to_value(event)
        .and_then(serde_json::from_value::<Json>)
        .unwrap_or(Json::Null)
}

fn completed_logging_json(state: &ResponsesStreamBaseState) -> Json {
    state
        .completed_response
        .as_ref()
        .map_or(Json::Null, |event| {
            logging_event(event, &state.hidden_params, &state.raw_response_headers)
        })
}

impl ResponsesStreamHooks<'_> {
    pub async fn log_completed_response(
        &self,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        details: &IndexMap<String, Json>,
    ) -> Result<(), PortFailure> {
        if state.completed_response_logged {
            return Ok(());
        }
        state.completed_response_logged = true;
        if state.persist_before_logging {
            self.persist_cache(state)?;
        }
        let end_time = self.clock.now();
        let request = SuccessRequest {
            result: SuccessResult::Other(completed_logging_json(state)),
            start_time: Some(state.start_time),
            end_time: Some(end_time),
            cache_hit: state.completed_response_cache_hit,
            prefer_async_handlers: false,
            standard_logging_object: None,
            guardrail_snapshot: None,
        };
        if let Some(success) = self.success.as_ref() {
            success.async_success_handler(&request).await;
            success.success_handler(&request);
        }
        self.run_post_success_hooks(state, callback, details, call_type.as_deref(), end_time)
            .await;
        Ok(())
    }

    pub async fn log_completed_response_async(
        &self,
        state: &mut ResponsesStreamBaseState,
        callback: &mut ResponsesStreamHookState,
        details: &IndexMap<String, Json>,
    ) -> Result<(), PortFailure> {
        if state.completed_response_logged {
            return Ok(());
        }
        state.completed_response_logged = true;
        if state.persist_before_logging {
            self.schedule_cache(state)?;
        }
        let end_time = self.clock.now();
        let request = SuccessRequest {
            result: SuccessResult::Other(completed_logging_json(state)),
            start_time: Some(state.start_time),
            end_time: Some(end_time),
            cache_hit: state.completed_response_cache_hit,
            prefer_async_handlers: true,
            standard_logging_object: None,
            guardrail_snapshot: None,
        };
        if let Some(success) = self.success.as_ref().map(Arc::clone) {
            if success.deferred_armed() {
                callback.deferred_stream_complete_args = Some(request);
            } else {
                let dispatch = request.clone();
                let port = Arc::clone(&success);
                let work = async move {
                    port.dispatch_success_handlers(dispatch).await;
                    Ok::<(), PortFailure>(())
                };
                if spawn_logging_task(&state.pending_tasks, SUCCESS_DISPATCH_TASK, work).is_none() {
                    success.dispatch_success_handlers(request).await;
                }
            }
        }
        self.run_post_success_hooks(state, callback, details, call_type.as_deref(), end_time)
            .await;
        Ok(())
    }
}
