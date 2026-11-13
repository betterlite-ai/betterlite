use std::borrow::Cow;
use std::time::SystemTime;

use betterlite_logging::otel::fan_out::attach::sink_key;
use betterlite_logging::otel::fan_out::attributes::tenant_view;
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
use serde_json::{Value as Json, json};

use crate::helpers::{Helper, HelperError, HelperResult};

use input::{Input, SpanInput};
use values::{attributes_of, view_json};

mod input;
mod values;

const TRACE_ID: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10;
const SPAN_ID: u64 = 0x0102_0304_0506_0708;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let value = match input.function.as_str() {
            "for_destination" => view_json(&tenant_view(&span_from(&input.span)?)),
            "sink_key" => sink_record(input),
            other => {
                return Err(Box::new(HelperError {
                    class: "UnknownFunction".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: format!("unknown b3849 function {other}"),
                }));
            }
        };
        Ok(json!({"result": value}))
    }
}

fn span_from(input: &SpanInput) -> Result<SpanData, Box<HelperError>> {
    Ok(SpanData {
        span_context: SpanContext::new(
            TraceId::from(TRACE_ID),
            SpanId::from(SPAN_ID),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Owned(input.name.clone()),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH,
        attributes: attributes_of(&input.attributes)?,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::builder("b3849").build(),
    })
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
