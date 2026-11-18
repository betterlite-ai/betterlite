use std::cell::Cell;
use std::collections::BTreeMap;

use betterlite_core::api::response_synthetic_events::billing::{
    BilledTerminal, billed_terminal_object, billed_terminal_response,
};
// ast-grep-ignore: no-long-names
use betterlite_core::api::response_synthetic_events::builder::build_synthetic_response_events as build_events;
use betterlite_core::api::response_synthetic_events::copies::{
    LoggedCopy, LoggingEvent, logging_copy,
};
use betterlite_core::api::response_synthetic_events::error::SyntheticEventsError;
use betterlite_core::api::response_synthetic_events::model_dump::dump_json_safely;
use betterlite_core::api::response_synthetic_events::stamping::stamp_object_usage_cost;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, ResponseEvent};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::{ResponseApiUsage, ResponsesApiResponse};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use super::probes::{CostProbe, FixedEstimate};

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Row {
    kind: String,
    response: Value,
    chunk_size: i64,
    cost: Option<CostSpec>,
    estimate: Option<EstimateSpec>,
    event: Value,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum CostSpec {
    Value { cost: String },
    Null,
    Raise,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum EstimateSpec {
    Value {
        input_tokens: i64,
        output_tokens: i64,
    },
    Empty,
}

pub(super) fn rows_answer(rows: &[Row], answer: fn(&Row) -> HelperResult) -> HelperResult {
    rows.iter()
        .map(answer)
        .collect::<Result<Vec<Value>, _>>()
        .map(|answers| json!({"rows": answers}))
}

pub(super) fn build_answer(row: &Row) -> HelperResult {
    let mut response: ResponsesApiResponse = serde_json::from_value(row.response.clone())
        .map_err(|error| value_error(error.to_string()))?;
    let probe = CostProbe::new(row.cost.as_ref())?;
    let outcome = build_events(&mut response, probe.as_ref(), row.chunk_size);
    Ok(build_report(outcome, probe.as_ref()))
}

fn build_report(
    outcome: Result<Vec<ResponsesAPIStreamingResponse>, SyntheticEventsError>,
    probe: Option<&CostProbe>,
) -> Value {
    let calls = probe.map_or(0, |held| held.calls.get());
    let (raised, message, events) = match outcome {
        Ok(events) => (false, Value::Null, json!(events)),
        Err(error) => (true, json!(error.to_string()), Value::Null),
    };
    json!({"raised": raised, "message": message, "events": events, "cost_calls": calls})
}

pub(super) fn terminal_answer(row: &Row) -> HelperResult {
    let estimate = estimate_probe(row.estimate.as_ref());
    if row.kind == "dict" || row.kind == "other" {
        return terminal_object_answer(row, estimate.as_ref());
    }
    terminal_model_answer(row, estimate.as_ref())
}

fn terminal_object_answer(row: &Row, estimate: Option<&FixedEstimate>) -> HelperResult {
    let object = serde_json::from_value(row.response.clone()).unwrap_or(Json::Null);
    let constructed = billed_terminal_object(&object, estimate);
    let calls = estimate.map_or(0, |held| held.calls.get());
    Ok(json!({"same": false, "response": constructed, "estimate_calls": calls}))
}

fn terminal_model_answer(row: &Row, estimate: Option<&FixedEstimate>) -> HelperResult {
    let response: ResponsesApiResponse = serde_json::from_value(row.response.clone())
        .map_err(|error| value_error(error.to_string()))?;
    let billed = billed_terminal_response(&response, estimate);
    let (same, terminal) = match &billed {
        BilledTerminal::Same(terminal) => (true, *terminal),
        BilledTerminal::Estimated(terminal) => (false, terminal.as_ref()),
    };
    let calls = estimate.map_or(0, |held| held.calls.get());
    Ok(json!({"same": same, "response": terminal, "estimate_calls": calls}))
}

pub(super) fn stamp_answer(row: &Row) -> HelperResult {
    let probe = CostProbe::new(row.cost.as_ref())?;
    let response = serde_json::from_value(row.response.clone()).unwrap_or(Json::Null);
    let Some(mut constructed) = billed_terminal_object(&response, None::<&FixedEstimate>) else {
        return Ok(json!({"response": null, "cost_calls": 0}));
    };
    stamp_object_usage_cost(&mut constructed, probe.as_ref());
    let calls = probe.as_ref().map_or(0, |held| held.calls.get());
    Ok(json!({"response": constructed, "cost_calls": calls}))
}

pub(super) fn copy_answer(row: &Row) -> HelperResult {
    match row.kind.as_str() {
        "value" => Ok(copy_value(row)),
        "completed" => copy_completed(row),
        _ => copy_generic(row),
    }
}

fn copy_value(row: &Row) -> Value {
    let value = serde_json::from_value(row.event.clone()).unwrap_or(Json::Null);
    let copied = logging_copy(LoggingEvent::Value(&value));
    json!({"same": matches!(copied, LoggedCopy::Same(_)), "original": value, "copy": copy_json(&copied)})
}

fn copy_completed(row: &Row) -> HelperResult {
    let response: ResponsesApiResponse = serde_json::from_value(row.response.clone())
        .map_err(|error| value_error(error.to_string()))?;
    let completed = ResponsesAPIStreamingResponse::Known(Box::new(
        KnownResponseEvent::ResponseCompleted(ResponseEvent {
            response,
            extra: Extra::new(),
        }),
    ));
    let mut copied = logging_copy(LoggingEvent::Event(&completed));
    set_copy_cost(&mut copied);
    Ok(json!({"same": false, "original": json!(completed), "copy": copy_json(&copied)}))
}

fn copy_generic(row: &Row) -> HelperResult {
    let event: ResponsesAPIStreamingResponse = serde_json::from_value(row.event.clone())
        .map_err(|error| value_error(error.to_string()))?;
    let copied = logging_copy(LoggingEvent::Event(&event));
    Ok(json!({"same": false, "original": json!(event), "copy": copy_json(&copied)}))
}

fn copy_json(copied: &LoggedCopy<'_>) -> Value {
    match copied {
        LoggedCopy::Same(value) => json!(value),
        LoggedCopy::Detached(event) => json!(event),
    }
}

pub(super) fn dump_answer(row: &Row) -> HelperResult {
    let response: ResponsesApiResponse = serde_json::from_value(row.response.clone())
        .map_err(|error| value_error(error.to_string()))?;
    Ok(match dump_json_safely(&response) {
        Some(text) => {
            json!({"dumped": serde_json::from_str::<Value>(&text).map_err(|error| value_error(error.to_string()))?})
        }
        None => json!({"dumped": null}),
    })
}

fn estimate_probe(spec: Option<&EstimateSpec>) -> Option<FixedEstimate> {
    let given = spec?;
    let answer = match given {
        EstimateSpec::Value {
            input_tokens,
            output_tokens,
        } => Some(ResponseApiUsage {
            input_tokens: *input_tokens,
            input_tokens_details: None,
            output_tokens: *output_tokens,
            output_tokens_details: None,
            total_tokens: input_tokens.saturating_add(*output_tokens),
            cost: None,
            extra: BTreeMap::new(),
        }),
        EstimateSpec::Empty => None,
    };
    Some(FixedEstimate {
        answer,
        calls: Cell::new(0),
    })
}

fn set_copy_cost(copied: &mut LoggedCopy<'_>) {
    if let LoggedCopy::Detached(event) = copied
        && let ResponsesAPIStreamingResponse::Known(known) = event.as_mut()
        && let KnownResponseEvent::ResponseCompleted(completed) = known.as_mut()
        && let Some(usage) = completed.response.usage.as_mut()
    {
        usage.cost = Some(Decimal::new(25, 2));
    }
}

pub(super) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
