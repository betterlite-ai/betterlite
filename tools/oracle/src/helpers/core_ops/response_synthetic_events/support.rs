use betterlite_core::api::response_synthetic_events::billing::{
    BilledTerminal, UsageEstimate, billed_terminal_object, billed_terminal_response,
};
use betterlite_core::api::response_synthetic_events::builder::build_synthetic_response_events;
use betterlite_core::api::response_synthetic_events::copies::{
    LoggedCopy, LoggingEvent, logging_copy,
};
use betterlite_core::api::response_synthetic_events::model_dump::dump_json_safely;
use betterlite_core::api::response_synthetic_events::stamping::stamp_object_usage_cost;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, ResponseEvent};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::probes::{CostProbe, FixedEstimate};

#[derive(Deserialize)]
pub(super) struct Row {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    response: Value,
    #[serde(default)]
    chunk_size: i64,
    cost: Option<CostSpec>,
    estimate: Option<EstimateSpec>,
    #[serde(default)]
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

pub(super) fn rows_answer<Row, Build>(rows: &[Row], build: Build) -> HelperResult
where
    Build: Fn(&Row) -> HelperResult,
{
    let answers: Vec<Value> = rows.iter().map(build).collect::<Result<_, _>>()?;
    Ok(json!({"rows": answers}))
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

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, Box<HelperError>> {
    serde_json::from_value(value.clone()).map_err(|error| value_error(error.to_string()))
}

fn json_from(value: &Value) -> Json {
    serde_json::from_value(value.clone()).unwrap_or(Json::Null)
}

fn as_estimate(estimate: &FixedEstimate) -> &dyn UsageEstimate {
    estimate
}

pub(super) fn build_row(row: &Row) -> HelperResult {
    let mut response: ResponsesApiResponse = decode(&row.response)?;
    let probe = CostProbe::new(row.cost.as_ref())?;
    let outcome = build_synthetic_response_events(&mut response, probe.as_ref(), row.chunk_size);
    let calls = probe.as_ref().map_or(0, |probe| probe.calls.get());
    Ok(match outcome {
        Ok(events) => {
            json!({"raised": false, "message": Value::Null, "events": events, "cost_calls": calls})
        }
        Err(error) => json!({
            "raised": true,
            "message": error.to_string(),
            "events": Value::Null,
            "cost_calls": calls,
        }),
    })
}

pub(super) fn terminal_row(row: &Row) -> HelperResult {
    let estimate = FixedEstimate::new(row.estimate.as_ref());
    let calls = estimate.as_ref().map_or(0, |estimate| estimate.calls.get());
    if row.kind == "other" {
        return Ok(json!({"same": false, "response": Value::Null, "estimate_calls": calls}));
    }
    if row.kind == "dict" {
        let response = json_from(&row.response);
        let constructed = billed_terminal_object(&response, estimate.as_ref().map(as_estimate));
        return Ok(json!({"same": false, "response": constructed, "estimate_calls": calls}));
    }
    let response: ResponsesApiResponse = decode(&row.response)?;
    let outcome = billed_terminal_response(&response, estimate.as_ref().map(as_estimate));
    let (same, terminal) = match outcome {
        BilledTerminal::Same(response) => (true, response),
        BilledTerminal::Estimated(response) => (false, response.as_ref()),
    };
    Ok(json!({"same": same, "response": terminal, "estimate_calls": calls}))
}

pub(super) fn stamp_row(row: &Row) -> HelperResult {
    let probe = CostProbe::new(row.cost.as_ref())?;
    let response = json_from(&row.response);
    let Some(mut constructed) = billed_terminal_object(&response, None) else {
        return Ok(json!({"response": Value::Null, "cost_calls": 0}));
    };
    stamp_object_usage_cost(&mut constructed, probe.as_ref());
    Ok(
        json!({"response": constructed, "cost_calls": probe.as_ref().map_or(0, |probe| probe.calls.get())}),
    )
}

pub(super) fn copy_row(row: &Row) -> HelperResult {
    let value = json_from(&row.event);
    let event = match row.kind.as_str() {
        "value" => None,
        "completed" => Some(completed_event(&row.response)?),
        _ => Some(decode::<ResponsesAPIStreamingResponse>(&row.event)?),
    };
    let mut copied = match &event {
        Some(event) => logging_copy(LoggingEvent::Event(event)),
        None => logging_copy(LoggingEvent::Value(&value)),
    };
    let same = matches!(copied, LoggedCopy::Same(_));
    set_copy_cost(&mut copied);
    let copied_json = match &copied {
        LoggedCopy::Same(value) => json!(value),
        LoggedCopy::Detached(event) => json!(event),
    };
    let original = match &event {
        Some(event) => json!(event),
        None => json!(value),
    };
    Ok(json!({"same": same, "original": original, "copy": copied_json}))
}

fn completed_event(response: &Value) -> Result<ResponsesAPIStreamingResponse, Box<HelperError>> {
    let response: ResponsesApiResponse = decode(response)?;
    Ok(ResponsesAPIStreamingResponse::Known(Box::new(
        KnownResponseEvent::ResponseCompleted(ResponseEvent {
            response,
            extra: Extra::new(),
        }),
    )))
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

pub(super) fn dump_row(row: &Row) -> HelperResult {
    let response: ResponsesApiResponse = decode(&row.response)?;
    let dumped = match dump_json_safely(&response) {
        Some(text) => {
            serde_json::from_str::<Value>(&text).map_err(|error| value_error(error.to_string()))?
        }
        None => Value::Null,
    };
    Ok(json!({"dumped": dumped}))
}
