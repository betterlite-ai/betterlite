use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::context::post_response::ServiceSpanTiming;
use betterlite_logging::otel::v2_callback::registered;
use betterlite_logging::otel::v2_callback::registry::{CallbackList, V2Registry};
use betterlite_logging::otel::v2_callback::service::ServiceHook;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use opentelemetry::trace::{SpanId, Status};
use opentelemetry_sdk::trace::SpanData as SdkSpanData;
use serde_json::{Value, json};

use self::harness::build;

mod harness;
mod tests;

const START: f64 = 1_704_067_200.0;
const END: f64 = 1_704_067_201.5;

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Function {
    Init,
    Registration,
    Service,
    Guardrail,
    Precall,
    Phase,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    call_type: String,
    error: Option<String>,
    metrics_only: bool,
    status: u16,
    second_status: u16,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Init => init(),
            Function::Registration => registration(),
            Function::Service => service(input),
            Function::Guardrail => guardrail(),
            Function::Precall => precall(input),
            Function::Phase => phase(),
        }
    }
}

fn init() -> HelperResult {
    let registry = V2Registry::default();
    let _built = build(&registry)?;
    Ok(answer(&registry, false, false, &[]))
}

fn registration() -> HelperResult {
    let registry = V2Registry::default();
    let _first = build(&registry)?;
    let _second = build(&registry)?;
    Ok(answer(&registry, false, false, &[]))
}

fn service(input: &Input) -> HelperResult {
    let registry = V2Registry::default();
    let built = build(&registry)?;
    let root = built.span("root");
    let payload = ServiceLoggerPayload {
        service: if input.metrics_only {
            ServiceType::SelfService
        } else {
            ServiceType::Redis
        },
        call_type: input.call_type.clone(),
        caller: Some("test".to_owned()),
        error: None,
    };
    let hook = ServiceHook {
        payload: &payload,
        parent_otel_span: Some(&root.context),
        start_time: Some(at(START)?),
        timing: ServiceSpanTiming {
            end_time: Some(at(END)?),
            parent_end_time: None,
            post_response: false,
        },
        event_metadata: None,
    };
    match input.error.as_deref() {
        Some(text) => built.callback.async_service_failure_hook(&hook, Some(text)),
        None => built.callback.async_service_success_hook(&hook),
    }
    let spans = built.finished()?;
    Ok(answer(
        &registry,
        false,
        false,
        &records(&spans, root.context.span_id()),
    ))
}

fn guardrail() -> HelperResult {
    let registry = V2Registry::default();
    let first = build(&registry)?;
    let _second = build(&registry)?;
    let root = first.span("root");
    let mut trace = RequestTraceContext::default();
    first
        .callback
        .mark_request_root_span(&root, None, &mut trace);
    registered::emit_guardrail_span(&registry, &entry()?, &trace);
    let spans = first.finished()?;
    Ok(answer(
        &registry,
        false,
        false,
        &records(&spans, root.context.span_id()),
    ))
}

fn precall(input: &Input) -> HelperResult {
    let registry = V2Registry::default();
    let built = build(&registry)?;
    let root = built.span("root");
    let facts = ExceptionFacts {
        class_name: "AuthenticationError".to_owned(),
        message: Some("litellm.AuthenticationError: bad credentials".to_owned()),
        status_code: Some(401),
        llm_provider: Some("openai".to_owned()),
        ..ExceptionFacts::default()
    };
    built
        .callback
        .record_error_attributes_on_span(Some(&root), Some(&facts), input.status);
    if input.second_status > 0 {
        built.callback.record_error_attributes_on_span(
            Some(&root),
            Some(&facts),
            input.second_status,
        );
    }
    root.end();
    let spans = built.finished()?;
    Ok(answer(
        &registry,
        false,
        false,
        &records(&spans, root.context.span_id()),
    ))
}

fn phase() -> HelperResult {
    let registry = V2Registry::default();
    let before = registered::phase_span(&registry, "auth").is_some();
    let built = build(&registry)?;
    let routed = registered::phase_span(&registry, "auth").is_some();
    let spans = built.finished()?;
    Ok(answer(
        &registry,
        before,
        routed,
        &records(&spans, SpanId::INVALID),
    ))
}

fn answer(registry: &V2Registry, phase_before: bool, phase_routed: bool, spans: &[Value]) -> Value {
    json!({
        "constructed": true,
        "service_callbacks": registry.entries(CallbackList::Service).len(),
        "input_callbacks": registry.entries(CallbackList::Input).len(),
        "phase_before": phase_before,
        "phase_routed": phase_routed,
        "spans": spans,
    })
}

fn records(spans: &[SdkSpanData], root: SpanId) -> Vec<Value> {
    spans.iter().map(|span| record(span, root)).collect()
}

fn record(span: &SdkSpanData, root: SpanId) -> Value {
    let parent = if span.parent_span_id == SpanId::INVALID {
        "none"
    } else if span.parent_span_id == root {
        "root"
    } else {
        "other"
    };
    json!({
        "name": span.name.as_ref(),
        "parent": parent,
        "status": match &span.status {
            Status::Unset => "unset",
            Status::Ok => "ok",
            Status::Error { .. } => "error",
        },
        "events": span.events.len(),
        "error_type": attribute(span, "error.type"),
        "error_message": attribute(span, "error.message"),
        "error_code": attribute(span, "litellm.provider.error.code"),
        "service": attribute(span, "litellm.service.name"),
        "guardrail": attribute(span, "litellm.guardrail.name"),
    })
}

fn attribute(span: &SdkSpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| pair.value.to_string())
}

fn entry() -> Result<BTreeMap<String, OtelModelValue>, Box<HelperError>> {
    let value = json!({
        "guardrail_name": "pii",
        "guardrail_status": "success",
        "start_time": START,
        "end_time": END,
    });
    serde_json::from_value(value).or_value_error()
}

fn at(seconds: f64) -> Result<SystemTime, Box<HelperError>> {
    let elapsed = Duration::try_from_secs_f64(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(elapsed)
        .ok_or("the instant is outside the representable range")
        .or_value_error()
}
