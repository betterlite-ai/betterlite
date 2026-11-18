#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, OutputTextDeltaEvent};
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::{ResponseApiUsage, ResponsesApiResponse};
use rust_decimal::Decimal;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::billing::{
    BilledTerminal, UsageEstimate, UsageSource, billed_terminal_object, billed_terminal_response,
    usage_as_model,
};
use super::builder::build_synthetic_response_events;
use super::chunks::{EventBudget, MAX_SYNTHETIC_BYTES, MAX_SYNTHETIC_EVENTS};
use super::constants::TOOL_ARGUMENTS_DELTA_EVENTS;
use super::copies::{LoggedCopy, LoggingEvent, logging_copy};
use super::error::SyntheticEventsError;
use super::estimate::{
    EstimateRequest, TokenCountRequest, UsageTokenization, estimate_usage_from_text,
    estimate_usage_safely,
};
use super::model_dump::{HasModelDumpJson, ResponseObject, dump_json_safely, dump_response_object};
use super::python_value::{has_content, text_or_empty};
use super::stamping::{
    CostTarget, ResponsesCostCalculator, stamp_object_usage_cost, stamp_usage_cost,
};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

impl Capture {
    fn messages(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if !visitor.0.is_empty() {
            self.0.lock().unwrap().push(visitor.0);
        }
    }
}

fn capture<T, Call>(call: Call) -> (T, Vec<String>)
where
    Call: FnOnce() -> T,
{
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    let value = call();
    drop(guard);
    (value, captured.messages())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    parse(text)
}

fn response_with(output: &str, extra: &str) -> ResponsesApiResponse {
    parse(&format!(
        r#"{{"id":"resp_a","created_at":1700000000,"status":"completed","output":{output}{extra}}}"#
    ))
}

fn response(output: &str) -> ResponsesApiResponse {
    response_with(output, "")
}

fn to_json<T: serde::Serialize>(value: &T) -> Json {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn events_json(events: &[ResponsesAPIStreamingResponse]) -> Vec<Json> {
    match to_json(&events) {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            panic!("serialized events are a list")
        }
    }
}

fn field<'a>(value: &'a Json, key: &str) -> &'a Json {
    match value {
        Json::Object(fields) => fields.get(key).unwrap_or(&Json::Null),
        _ => &Json::Null,
    }
}

fn at<'a>(value: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(value, |current, key| field(current, key))
}

fn item_at(value: &Json, index: usize) -> &Json {
    match value {
        Json::List(items) => items.get(index).unwrap_or(&Json::Null),
        _ => &Json::Null,
    }
}

fn kind(value: &Json) -> &str {
    match field(value, "type") {
        Json::Text(text) => text,
        _ => "",
    }
}

fn text(value: &Json) -> &str {
    match value {
        Json::Text(text) => text,
        _ => "",
    }
}

fn integer(value: &Json) -> i64 {
    match value {
        Json::Number(number) => number.as_i64().unwrap_or(i64::MIN),
        _ => i64::MIN,
    }
}

fn number_text(value: &Json) -> String {
    match value {
        Json::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn has_key(value: &Json, key: &str) -> bool {
    matches!(value, Json::Object(fields) if fields.contains_key(key))
}

fn kinds(events: &[Json]) -> Vec<String> {
    events.iter().map(|event| kind(event).to_owned()).collect()
}

fn deltas(events: &[Json], event_type: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| kind(event) == event_type)
        .map(|event| text(field(event, "delta")).to_owned())
        .collect()
}

fn event_of<'a>(events: &'a [Json], event_type: &str) -> &'a Json {
    events
        .iter()
        .find(|event| kind(event) == event_type)
        .expect("event present")
}

fn events_of<'a>(events: &'a [Json], event_type: &str) -> Vec<&'a Json> {
    events
        .iter()
        .filter(|event| kind(event) == event_type)
        .collect()
}

fn position(events: &[Json], event_type: &str) -> usize {
    events
        .iter()
        .position(|event| kind(event) == event_type)
        .expect("event present")
}

fn build(
    transformed: &mut ResponsesApiResponse,
    calculator: Option<&FixedCost>,
    chunk_size: i64,
) -> Vec<Json> {
    let events = build_synthetic_response_events(transformed, calculator, chunk_size).unwrap();
    events_json(&events)
}

fn text_part(text_value: &str) -> String {
    format!(r#"{{"type":"output_text","text":{text_value},"annotations":[],"logprobs":null}}"#)
}

fn message_item(id: &str, content: &str) -> String {
    format!(
        r#"{{"type":"message","id":"{id}","status":"completed","role":"assistant","content":{content}}}"#
    )
}

fn message_with_text(id: &str, text_value: &str) -> String {
    message_item(
        id,
        &format!("[{}]", text_part(&format!("\"{text_value}\""))),
    )
}

#[derive(Debug, thiserror::Error)]
#[error("cost lookup failed")]
struct CostFailure;

struct FixedCost {
    cost: Option<Decimal>,
    fails: bool,
    calls: Mutex<Vec<String>>,
}

impl FixedCost {
    fn returning(cost: Decimal) -> Self {
        Self {
            cost: Some(cost),
            fails: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn silent() -> Self {
        Self {
            cost: None,
            fails: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            cost: None,
            fails: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

fn cost_label(target: &CostTarget<'_>) -> String {
    match target {
        CostTarget::Model(response) => response.id.clone(),
        CostTarget::Object(fields) => match fields.get("id") {
            Some(value) => text(value).to_owned(),
            None => String::new(),
        },
    }
}

impl ResponsesCostCalculator for FixedCost {
    type Error = CostFailure;

    fn response_cost(&self, target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        self.calls.lock().unwrap().push(cost_label(target));
        if self.fails {
            return Err(CostFailure);
        }
        Ok(self.cost)
    }
}

fn usage(input_tokens: i64, output_tokens: i64) -> ResponseApiUsage {
    ResponseApiUsage {
        input_tokens,
        input_tokens_details: None,
        output_tokens,
        output_tokens_details: None,
        total_tokens: input_tokens.saturating_add(output_tokens),
        cost: None,
        extra: BTreeMap::new(),
    }
}

struct FixedEstimate {
    usage: Option<ResponseApiUsage>,
    calls: Mutex<usize>,
}

impl FixedEstimate {
    fn returning(input_tokens: i64, output_tokens: i64) -> Self {
        Self {
            usage: Some(usage(input_tokens, output_tokens)),
            calls: Mutex::new(0),
        }
    }

    fn empty() -> Self {
        Self {
            usage: None,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl UsageEstimate for FixedEstimate {
    fn estimate(&self) -> Option<ResponseApiUsage> {
        let mut calls = self.calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        self.usage.clone()
    }
}

struct RecordedCount {
    model: String,
    messages: Option<Vec<Json>>,
    text: Option<String>,
    count_response_tokens: bool,
}

struct FixedTokenization {
    messages: Vec<Json>,
    fails: bool,
    counts: Mutex<Vec<RecordedCount>>,
    transforms: Mutex<Vec<(Json, Json)>>,
}

impl FixedTokenization {
    fn working() -> Self {
        Self {
            messages: vec![json(r#"{"role":"user","content":"hello"}"#)],
            fails: false,
            counts: Mutex::new(Vec::new()),
            transforms: Mutex::new(Vec::new()),
        }
    }

    fn counts(&self) -> Vec<RecordedCount> {
        std::mem::take(&mut *self.counts.lock().unwrap())
    }

    fn transforms(&self) -> Vec<(Json, Json)> {
        std::mem::take(&mut *self.transforms.lock().unwrap())
    }
}

impl UsageTokenization for FixedTokenization {
    type Error = CostFailure;

    fn messages(&self, request: &EstimateRequest<'_>) -> Result<Vec<Json>, Self::Error> {
        if self.fails {
            return Err(CostFailure);
        }
        self.transforms.lock().unwrap().push((
            request.request_input.clone(),
            request.responses_api_request.clone(),
        ));
        Ok(self.messages.clone())
    }

    fn count(&self, request: TokenCountRequest<'_>) -> Result<i64, Self::Error> {
        self.counts.lock().unwrap().push(RecordedCount {
            model: request.model.to_owned(),
            messages: request.messages.map(<[Json]>::to_vec),
            text: request.text.map(str::to_owned),
            count_response_tokens: request.count_response_tokens,
        });
        Ok(if request.messages.is_some() { 3 } else { 4 })
    }
}

struct BrokenResponse;

impl HasModelDumpJson for BrokenResponse {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        let source = serde_json::from_str::<u8>("not a number").unwrap_err();
        Err(SyntheticEventsError::Payload(source))
    }
}

mod billing;
mod copies;
mod estimate;
mod items;
mod lifecycle;
mod parts;
mod stamping;
