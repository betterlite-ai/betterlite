use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::SystemTime;

use betterlite_logging::otel::config::destination::{
    DestinationFields, OtelDestination, SpanScope,
};
use betterlite_logging::otel::fan_out::attach::{credential_name, sink_key};
use betterlite_logging::otel::fan_out::attributes::{
    attribute_map, in_scope, is_excluded_database_span, is_llm_call_span, scoped, tenant_view,
};
use betterlite_logging::otel::fan_out::destination::destination_resource;
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::{
    Event, SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use serde_json::{Value as Json, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use input::{DestinationInput, Input, SpanInput, StatusInput};
use values::{attributes_of, error, resource_json, resource_of, view_json};

mod input;
mod values;

const TRACE_ID: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
const SPAN_ID: u64 = 0x0102_0304_0506_0708;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let value = match input.function.as_str() {
            "for_destination" => for_destination_record(input)?,
            "scoped" => {
                let span = span_from(&input.span)?;
                view_json(&scoped(span, span_scope(input.span_scope.as_deref())))
            }
            "in_scope" => json!(in_scope(
                &span_from(&input.span)?,
                span_scope(input.span_scope.as_deref())
            )),
            "is_llm_call_span" => json!(is_llm_call_span(&span_from(&input.span)?)),
            "is_excluded_database_span" => excluded_record(input)?,
            "sink_key" => sink_record(input),
            "credential_name" => json!(credential_name(&input.header)),
            other => {
                return Err(error(
                    "UnknownFunction",
                    format!("unknown b3849 function {other}"),
                ));
            }
        };
        Ok(json!({"result": value}))
    }
}

fn span_from(input: &SpanInput) -> Result<SpanData, Box<HelperError>> {
    let parent = match input.parent.as_deref() {
        Some(hex) => SpanId::from_hex(hex)
            .map_err(|failure| error("InvalidInput", format!("bad parent id {hex}: {failure}")))?,
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

fn status_of(status: &StatusInput) -> Status {
    match status.code.as_str() {
        "ok" => Status::Ok,
        "error" => Status::Error {
            description: Cow::Owned(status.description.clone().unwrap_or_default()),
        },
        _ => Status::Unset,
    }
}

fn span_scope(value: Option<&str>) -> SpanScope {
    if value == Some("llm_only") {
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
        span_scope: span_scope(input.span_scope.as_deref()),
    })
}

fn for_destination_record(input: &Input) -> Result<Json, Box<HelperError>> {
    let span = span_from(&input.span)?;
    let destination = destination_from(&input.destination);
    let view = tenant_view(&span);
    let resource = destination_resource(&resource_of(&input.span.resource), &destination);
    let Json::Object(mut record) = view_json(&view) else {
        return Err(error("InvalidInput", "a view is an object".to_owned()));
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

#[path = "b3849/tests.rs"]
mod tests;
