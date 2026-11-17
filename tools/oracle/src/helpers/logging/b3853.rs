use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::context::post_response::ServiceSpanTiming;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_logging::otel::v2_callback::service::{self, ServiceHook};
use betterlite_logging::otel::v2_callback::{proxy, registered};
use betterlite_logging::registry::phases::CallbackList;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use opentelemetry::trace::{Span as _, SpanId, Status};
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
            Function::Init => construction(1),
            Function::Registration => construction(2),
            Function::Service => service(input),
            Function::Guardrail => guardrail(),
            Function::Precall => precall(input),
            Function::Phase => phase(),
        }
    }
}

fn construction(times: usize) -> HelperResult {
    let registry = V2Registry::default();
    for _ in 0..times {
        build(&registry)?;
    }
    Ok(answer(&registry, false, false, &[], SpanId::INVALID))
}

fn service(input: &Input) -> HelperResult {
    let registry = V2Registry::default();
    let built = build(&registry)?;
    let root_id = built.root.context.span_id();
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
        parent_otel_span: Some(&built.root.context),
        start_time: Some(UNIX_EPOCH + Duration::from_secs_f64(START)),
        timing: ServiceSpanTiming {
            end_time: Some(UNIX_EPOCH + Duration::from_secs_f64(END)),
            parent_end_time: None,
            post_response: false,
        },
        event_metadata: None,
    };
    let _emitted = input.error.as_deref().map_or_else(
        || service::emit_service(&built.callback, &hook, None),
        |text| service::async_service_failure_hook(&built.callback, &hook, Some(text)),
    );
    let spans = built.spans.get_finished_spans().or_config_error()?;
    Ok(answer(&registry, false, false, &spans, root_id))
}

fn guardrail() -> HelperResult {
    let registry = V2Registry::default();
    let first = build(&registry)?;
    let _second = build(&registry)?;
    let root_id = first.root.context.span_id();
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&first.callback, &first.root, None, &mut trace);
    let entry: BTreeMap<String, OtelModelValue> = serde_json::from_value(json!({
        "guardrail_name": "pii",
        "guardrail_status": "success",
        "start_time": START,
        "end_time": END,
    }))
    .or_value_error()?;
    registered::emit_guardrail_span(&registry, &entry, &trace);
    let spans = first.spans.get_finished_spans().or_config_error()?;
    Ok(answer(&registry, false, false, &spans, root_id))
}

fn precall(input: &Input) -> HelperResult {
    let registry = V2Registry::default();
    let built = build(&registry)?;
    let root_id = built.root.context.span_id();
    let facts: ExceptionFacts = serde_json::from_value(json!({
        "class_name": "AuthenticationError",
        "message": "litellm.AuthenticationError: bad credentials",
        "status_code": 401,
        "llm_provider": "openai",
    }))
    .or_value_error()?;
    let callback = &built.callback;
    proxy::record_error_on_span(callback, Some(&built.root), Some(&facts), input.status);
    if input.second_status > 0 {
        proxy::record_error_on_span(
            callback,
            Some(&built.root),
            Some(&facts),
            input.second_status,
        );
    }
    built.root.lock().end();
    let spans = built.spans.get_finished_spans().or_config_error()?;
    Ok(answer(&registry, false, false, &spans, root_id))
}

fn phase() -> HelperResult {
    let registry = V2Registry::default();
    let trace = RequestTraceContext::default();
    let before = registered::phase_span(&registry, "auth", &trace).is_some();
    let built = build(&registry)?;
    let routed = registered::phase_span(&registry, "auth", &trace).is_some();
    let spans = built.spans.get_finished_spans().or_config_error()?;
    Ok(answer(&registry, before, routed, &spans, SpanId::INVALID))
}

fn answer(
    registry: &V2Registry,
    phase_before: bool,
    phase_routed: bool,
    spans: &[SdkSpanData],
    root: SpanId,
) -> Value {
    json!({
        "service_callbacks": registry.entries(CallbackList::Service).len(),
        "input_callbacks": registry.entries(CallbackList::Input).len(),
        "success_callbacks": registry.entries(CallbackList::Success).len(),
        "failure_callbacks": registry.entries(CallbackList::Failure).len(),
        "phase_before": phase_before,
        "phase_routed": phase_routed,
        "spans": spans.iter().map(|span| record(span, root)).collect::<Vec<Value>>(),
    })
}

fn record(span: &SdkSpanData, root: SpanId) -> Value {
    let parent = match span.parent_span_id {
        SpanId::INVALID => "none",
        id if id == root => "root",
        _ => "other",
    };
    let code = span
        .attributes
        .iter()
        .rfind(|pair| pair.key.as_str() == "litellm.provider.error.code")
        .map(|pair| pair.value.to_string());
    json!({
        "name": span.name.as_ref(),
        "parent": parent,
        "status": match &span.status {
            Status::Unset => "unset",
            Status::Ok => "ok",
            Status::Error { .. } => "error",
        },
        "provider_error_code": code,
        "events": span.events.len(),
        "kind": format!("{:?}", span.span_kind).to_ascii_lowercase(),
        "start": nanos(span.start_time).filter(|_| parent == "root"),
        "end": nanos(span.end_time).filter(|_| parent == "root"),
    })
}

// the span's epoch nanoseconds, as LiteLLM's to_ns reads the exporter's integer timestamps
fn nanos(time: SystemTime) -> Option<u64> {
    let elapsed = time.duration_since(UNIX_EPOCH).ok()?;
    let nanos = u64::try_from(elapsed.as_nanos()).ok()?;
    Some(nanos)
}
