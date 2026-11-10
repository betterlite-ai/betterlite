use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use betterlite_logging::otel::config::destination::{
    DestinationFields, OtelDestination, SpanScope,
};
use betterlite_logging::otel::fan_out::attach::{credential_name, sink_key};
use betterlite_logging::otel::fan_out::attributes::{
    attribute_map, in_scope, is_excluded_database_span, is_llm_call_span, scoped, tenant_view,
};
use betterlite_logging::otel::fan_out::destination::destination_resource;
use betterlite_logging::otel::mappers::base::AttributeMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::trace::{
    Event, SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{Array, InstrumentationScope, Key, KeyValue, StringValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const TRACE_ID: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
const SPAN_ID: u64 = 0x0102_0304_0506_0708;

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let value = match input.function.as_str() {
            "for_destination" => for_destination_record(input)?,
            "scoped" => {
                let span = span_from(&input.span)?;
                view_json(&scoped(span, scope_of(input)))
            }
            "in_scope" => json!(in_scope(&span_from(&input.span)?, scope_of(input))),
            "is_llm_call_span" => json!(is_llm_call_span(&span_from(&input.span)?)),
            "is_excluded_database_span" => excluded_record(input)?,
            "sink_key" => sink_record(input),
            "credential_name" => json!(credential_name(&input.header)),
            other => {
                return Err(helper_error(
                    "UnknownFunction",
                    format!("unknown b3849 function {other}"),
                ));
            }
        };
        Ok(json!({"result": value}))
    }
}

#[derive(Deserialize)]
pub(crate) struct Input {
    function: String,
    #[serde(default)]
    span: SpanInput,
    #[serde(default)]
    destination: DestinationInput,
    span_scope: Option<String>,
    endpoint: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    excluded: Vec<String>,
    #[serde(default)]
    header: String,
}

#[derive(Deserialize, Default)]
struct SpanInput {
    #[serde(default = "default_name")]
    name: String,
    #[serde(default)]
    attributes: BTreeMap<String, OtelModelValue>,
    #[serde(default)]
    events: Vec<EventInput>,
    #[serde(default)]
    status: StatusInput,
    parent: Option<String>,
    #[serde(default)]
    resource: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct EventInput {
    #[serde(default = "default_event_name")]
    name: String,
    #[serde(default)]
    attributes: BTreeMap<String, OtelModelValue>,
}

#[derive(Deserialize)]
struct StatusInput {
    code: String,
    description: Option<String>,
}

impl Default for StatusInput {
    fn default() -> Self {
        Self {
            code: "unset".to_owned(),
            description: None,
        }
    }
}

#[derive(Deserialize, Default)]
struct DestinationInput {
    endpoint: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    resource_attributes: BTreeMap<String, String>,
    callback_name: Option<String>,
    protocol: Option<String>,
    span_scope: Option<String>,
}

fn default_name() -> String {
    "call model".to_owned()
}

fn default_event_name() -> String {
    "exception".to_owned()
}

fn span_from(input: &SpanInput) -> Result<SpanData, Box<HelperError>> {
    let parent = match input.parent.as_deref() {
        Some(hex) => SpanId::from_hex(hex).map_err(|error| {
            helper_error("InvalidInput", format!("bad parent id {hex}: {error}"))
        })?,
        None => SpanId::INVALID,
    };
    let mut events = SpanEvents::default();
    for event in &input.events {
        events.events.push(Event::new(
            event.name.clone(),
            SystemTime::UNIX_EPOCH,
            attributes_of(&event.attributes)?,
            0,
        ));
    }
    Ok(SpanData {
        span_context: SpanContext::new(
            TraceId::from(TRACE_ID),
            SpanId::from(SPAN_ID),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: parent,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Owned(input.name.clone()),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH,
        attributes: attributes_of(&input.attributes)?,
        dropped_attributes_count: 0,
        events,
        links: SpanLinks::default(),
        status: status_of(&input.status),
        instrumentation_scope: InstrumentationScope::builder("b3849").build(),
    })
}

fn attributes_of(
    values: &BTreeMap<String, OtelModelValue>,
) -> Result<Vec<KeyValue>, Box<HelperError>> {
    let mut attributes = Vec::with_capacity(values.len());
    for (key, value) in values {
        attributes.push(KeyValue::new(Key::new(key.clone()), otel_value(value)?));
    }
    Ok(attributes)
}

fn otel_value(value: &OtelModelValue) -> Result<Value, Box<HelperError>> {
    match value {
        OtelModelValue::Bool(flag) => Ok(Value::Bool(*flag)),
        OtelModelValue::String(text) => Ok(Value::from(text.clone())),
        OtelModelValue::Number(number) => number
            .as_i64()
            .map(Value::I64)
            .or_else(|| number.as_f64().map(Value::F64))
            .ok_or_else(|| helper_error("InvalidInput", "unrepresentable number".to_owned())),
        OtelModelValue::Array(items) => {
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    OtelModelValue::String(text) => texts.push(StringValue::from(text.clone())),
                    OtelModelValue::Null(())
                    | OtelModelValue::Bool(_)
                    | OtelModelValue::Number(_)
                    | OtelModelValue::Array(_)
                    | OtelModelValue::Object(_) => {
                        return Err(helper_error(
                            "InvalidInput",
                            "only string arrays stand for OTel attribute arrays".to_owned(),
                        ));
                    }
                }
            }
            Ok(Value::Array(Array::String(texts)))
        }
        OtelModelValue::Null(()) | OtelModelValue::Object(_) => Err(helper_error(
            "InvalidInput",
            "unsupported span attribute value".to_owned(),
        )),
    }
}

fn status_of(status: &StatusInput) -> Status {
    match status.code.as_str() {
        "ok" => Status::Ok,
        "error" => Status::Error {
            description: Cow::Owned(status.description.clone().unwrap_or_default()),
        },
        _ => Status::Unset,
    }
}

fn scope_of(input: &Input) -> SpanScope {
    if input.span_scope.as_deref() == Some("llm_only") {
        SpanScope::LlmOnly
    } else {
        SpanScope::Full
    }
}

fn destination_from(input: &DestinationInput) -> OtelDestination {
    OtelDestination::from(DestinationFields {
        endpoint: input.endpoint.clone(),
        headers: input.headers.clone(),
        resource_attributes: input.resource_attributes.clone(),
        callback_name: input.callback_name.clone(),
        protocol: input.protocol.clone(),
        span_scope: if input.span_scope.as_deref() == Some("llm_only") {
            SpanScope::LlmOnly
        } else {
            SpanScope::Full
        },
    })
}

fn for_destination_record(input: &Input) -> Result<Json, Box<HelperError>> {
    let span = span_from(&input.span)?;
    let destination = destination_from(&input.destination);
    let view = tenant_view(&span);
    let resource = destination_resource(&resource_of(&input.span.resource), &destination);
    let Json::Object(mut record) = view_json(&view) else {
        return Err(helper_error(
            "InvalidInput",
            "a view is an object".to_owned(),
        ));
    };
    record.insert("resource".to_owned(), resource_json(&resource));
    Ok(Json::Object(record))
}

fn excluded_record(input: &Input) -> Result<Json, Box<HelperError>> {
    let span = span_from(&input.span)?;
    let excluded: BTreeSet<String> = input.excluded.iter().cloned().collect();
    Ok(json!(is_excluded_database_span(
        &attribute_map(&span.attributes),
        &excluded
    )))
}

fn sink_record(input: &Input) -> Json {
    match sink_key(input.endpoint.as_deref(), &input.headers) {
        Some((endpoint, credentials)) => {
            let pairs: Vec<Json> = credentials
                .iter()
                .map(|(name, value)| json!([name, value]))
                .collect();
            json!([endpoint, pairs])
        }
        None => Json::Null,
    }
}

fn resource_of(values: &BTreeMap<String, String>) -> Resource {
    Resource::builder_empty()
        .with_attributes(
            values.iter().map(|(key, value)| {
                KeyValue::new(Key::new(key.clone()), Value::from(value.clone()))
            }),
        )
        .build()
}

fn view_json(span: &SpanData) -> Json {
    json!({
        "name": span.name.as_ref(),
        "attributes": attributes_json(&attribute_map(&span.attributes)),
        "events": span.events.events.iter().map(event_json).collect::<Vec<Json>>(),
        "status": status_json(&span.status),
        "parent": parent_json(span.parent_span_id),
    })
}

fn attributes_json(attributes: &AttributeMap) -> Json {
    Json::Object(
        attributes
            .iter()
            .map(|(key, value)| (key.clone(), json_value(value)))
            .collect(),
    )
}

fn event_json(event: &Event) -> Json {
    let attributes: AttributeMap = event
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    json!({"name": event.name.as_ref(), "attributes": attributes_json(&attributes)})
}

fn status_json(status: &Status) -> Json {
    match status {
        Status::Unset => json!({"code": "unset", "description": ""}),
        Status::Ok => json!({"code": "ok", "description": ""}),
        Status::Error { description } => {
            json!({"code": "error", "description": description.as_ref()})
        }
    }
}

fn parent_json(parent: SpanId) -> Json {
    if parent == SpanId::INVALID {
        Json::Null
    } else {
        Json::String(format!("{parent:016x}"))
    }
}

fn resource_json(resource: &Resource) -> Json {
    Json::Object(
        resource
            .iter()
            .map(|(key, value)| (key.to_string(), json_value(value)))
            .collect(),
    )
}

fn json_value(value: &Value) -> Json {
    match value {
        Value::Bool(flag) => json!(flag),
        Value::I64(integer) => json!(integer),
        Value::F64(float) => json!(float),
        Value::String(text) => json!(text.as_str()),
        Value::Array(array) => match array {
            Array::Bool(flags) => json!(flags),
            Array::I64(integers) => json!(integers),
            Array::F64(floats) => json!(floats),
            Array::String(texts) => {
                json!(texts.iter().map(StringValue::as_str).collect::<Vec<&str>>())
            }
            _ => Json::Null,
        },
        _ => Json::Null,
    }
}

#[path = "b3849/tests.rs"]
mod tests;
