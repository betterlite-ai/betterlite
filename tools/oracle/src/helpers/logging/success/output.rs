use std::sync::Arc;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde_json::{Value, json};

use crate::compare::helper::HelperError;

use super::scenario::Scenario;
use super::support::{LoggedEvent, RecordingLogger};

#[derive(Serialize)]
pub(crate) struct EventOut {
    pub(crate) logger: String,
    pub(crate) event: String,
    pub(crate) stream: bool,
    pub(crate) model: String,
    pub(crate) messages: Value,
    pub(crate) response: Value,
}

#[derive(Serialize)]
pub(crate) struct DetailsOut {
    pub(crate) log_event_type: Option<String>,
    pub(crate) cache_hit: Option<bool>,
    pub(crate) completion_start_time: Option<String>,
    pub(crate) end_time: Option<String>,
}

pub(crate) fn events(loggers: &[Arc<RecordingLogger>]) -> Result<Vec<EventOut>, Box<HelperError>> {
    let mut output = Vec::new();
    for logger in loggers {
        for call in logger.calls() {
            output.push(event(&call)?);
        }
    }
    Ok(output)
}

fn event(call: &LoggedEvent) -> Result<EventOut, Box<HelperError>> {
    Ok(EventOut {
        logger: call.logger.clone(),
        event: call.event.clone(),
        stream: call.stream,
        model: call.model.clone(),
        messages: optional_json(call.messages.as_ref())?,
        response: optional_json(call.response.as_ref())?,
    })
}

pub(crate) fn optional_json(value: Option<&Json>) -> Result<Value, Box<HelperError>> {
    value.map_or(Ok(Value::Null), |present| {
        serde_json::to_value(present).map_err(|failure| super::value_error(&failure))
    })
}

pub(crate) fn details(state: &LoggingState) -> DetailsOut {
    DetailsOut {
        log_event_type: text_field(state, "log_event_type"),
        cache_hit: bool_field(state, "cache_hit"),
        completion_start_time: text_field(state, "completion_start_time"),
        end_time: text_field(state, "end_time"),
    }
}

fn text_field(state: &LoggingState, name: &str) -> Option<String> {
    match state.model_call_details.get(name) {
        Some(Json::Text(value)) => Some(value.clone()),
        _ => None,
    }
}

fn bool_field(state: &LoggingState, name: &str) -> Option<bool> {
    match state.model_call_details.get(name) {
        Some(Json::Bool(value)) => Some(*value),
        _ => None,
    }
}

pub(crate) fn handler_value(
    scenario: &Scenario,
    state: &LoggingState,
    context: &CorrelationContext,
) -> Result<Value, Box<HelperError>> {
    let counters: Vec<Vec<String>> = scenario
        .counter_calls
        .iter()
        .map(|counter| counter.calls())
        .collect();
    Ok(json!({
        "events": events(&scenario.loggers)?,
        "counters": counters,
        "context": {"trace_id": context.trace_id, "session_id": context.session_id},
        "details": details(state),
    }))
}

pub(crate) struct SyncOutcome {
    pub(crate) should_run: bool,
    pub(crate) ran: bool,
}

pub(crate) fn sync_value(
    scenario: &Scenario,
    outcome: &SyncOutcome,
) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "should_run": outcome.should_run,
        "ran": outcome.ran,
        "events": events(&scenario.loggers)?,
    }))
}

pub(crate) fn dispatch_value(scenario: &Scenario, marker: bool) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "events": events(&scenario.loggers)?,
        "marker": marker,
    }))
}
