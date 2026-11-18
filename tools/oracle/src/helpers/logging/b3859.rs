use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::otel::legacy_lifecycle::lifecycle::{
    LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall, instant_from_ns,
};
use betterlite_logging::otel::legacy_lifecycle::tracer::{DynamicOtel, TracerChoice};
use betterlite_logging::otel::legacy_policy::capture::LoggingFlags;
use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use betterlite_logging::otel::legacy_policy::span_time::{SpanTime, to_ns};
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::model::vocabulary::semconv::OTELSemconvCategory;
use betterlite_logging::otel::plumbing::exporters::HttpClients;
use betterlite_logging::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use betterlite_logging::otel_provider_assembly::legacy_otel_init::{
    LegacyOtel, LegacyOtelEnvironment, LegacyOtelInit, init_legacy_otel,
};
use betterlite_logging::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, Status, TraceContextExt as _, Tracer as _,
};
use opentelemetry::{Context, KeyValue};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use serde::Deserialize;
use serde_json::{Value, json};

mod harness;
mod tests;
use harness::{config, prepare};

pub(crate) struct Op;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Input {
    function: String,
    event: String,
    kwargs: MetadataMap,
    request_data: MetadataMap,
    start_time: f64,
    end_time: f64,
    callback_name: Option<String>,
    parent: Option<ParentInput>,
    dynamic_config: bool,
    dynamic_headers: Option<BTreeMap<String, String>>,
    config_headers: Option<String>,
    semconv: bool,
    repeat: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ParentInput {
    name: String,
    carrier: String,
    start: Option<f64>,
    end: Option<f64>,
}

struct ProbeDynamic {
    config: Option<OpenTelemetryConfig>,
    headers: Option<BTreeMap<String, String>>,
}

impl DynamicOtel for ProbeDynamic {
    fn construct_dynamic_otel_headers(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<BTreeMap<String, String>> {
        self.headers.clone()
    }

    fn construct_dynamic_otel_config(
        &self,
        _dynamic_params: &MetadataMap,
    ) -> Option<OpenTelemetryConfig> {
        self.config.clone()
    }
}

struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no connection is opened by this helper"))
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let exporter = InMemorySpanExporter::default();
        let roots = Arc::new(RootSpans::default());
        let (otel, parent) = prepare(input, &exporter, &roots).or_value_error()?;
        let lifecycle = LegacyLifecycle::from_otel(
            otel,
            LegacyLifecycleOptions {
                dynamic: Arc::new(ProbeDynamic {
                    config: input.dynamic_config.then(|| config(input)),
                    headers: input.dynamic_headers.clone(),
                }),
                live: Arc::clone(&roots),
                ..LegacyLifecycleOptions::default()
            },
        );
        let mut kwargs = input.kwargs.clone();
        if let Some(live) = &parent
            && input
                .parent
                .as_ref()
                .is_some_and(|spec| spec.carrier == "metadata")
        {
            insert_carrier(&mut kwargs, live);
        }
        let ambient = ambient_context(input, parent.as_ref());
        let guard = ambient.as_ref().map(Context::attach);
        let choice = match input.function.as_str() {
            "tracer" => lifecycle.tracer_choice(&kwargs).or_value_error()?,
            "guardrails" => guardrails(&lifecycle, input).or_value_error()?,
            _ => lifecycle_call(&lifecycle, input, &kwargs).or_value_error()?,
        };
        drop(guard);
        end_parent(input, parent.as_ref());
        observed(choice.branch(), &exporter).or_value_error()
    }
}

fn ambient_context(input: &Input, parent: Option<&LiveSpan>) -> Option<Context> {
    let spec = input.parent.as_ref()?;
    let live = parent?;
    (spec.carrier == "ambient")
        .then(|| Context::new().with_remote_span_context(live.context.clone()))
}

fn insert_carrier(kwargs: &mut MetadataMap, live: &LiveSpan) {
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        return;
    };
    let Some(OtelModelValue::Object(metadata)) = params.get_mut("metadata") else {
        return;
    };
    metadata.insert(
        "litellm_parent_otel_span".to_owned(),
        OtelModelValue::String(traceparent(&live.context)),
    );
}

fn traceparent(context: &SpanContext) -> String {
    let flags = if context.trace_flags().is_sampled() {
        "01"
    } else {
        "00"
    };
    format!(
        "00-{:032x}-{:016x}-{flags}",
        u128::from_be_bytes(context.trace_id().to_bytes()),
        u64::from_be_bytes(context.span_id().to_bytes())
    )
}

fn end_parent(input: &Input, parent: Option<&LiveSpan>) {
    let Some(live) = parent else {
        return;
    };
    let end = input
        .parent
        .as_ref()
        .and_then(|spec| spec.end)
        .unwrap_or(input.end_time);
    live.lock()
        .end_with_timestamp(instant_from_ns(to_ns(SpanTime::Seconds(end), 0)));
}

fn call_of<'call>(input: &Input, kwargs: &'call MetadataMap) -> LifecycleCall<'call> {
    LifecycleCall {
        kwargs,
        response: None,
        start_time: SpanTime::Seconds(input.start_time),
        end_time: SpanTime::Seconds(input.end_time),
        exception: None,
        flags: LoggingFlags {
            turn_off_message_logging: false,
            message_logging: true,
        },
    }
}

fn lifecycle_call(
    lifecycle: &LegacyLifecycle,
    input: &Input,
    kwargs: &MetadataMap,
) -> Result<TracerChoice, String> {
    let call = call_of(input, kwargs);
    let handle = if input.event == "failure" {
        LegacyLifecycle::handle_failure
    } else {
        LegacyLifecycle::handle_success
    };
    handle(lifecycle, &call).map_err(|error| error.to_string())?;
    if input.repeat {
        handle(lifecycle, &call).map_err(|error| error.to_string())?;
    }
    lifecycle
        .tracer_choice(kwargs)
        .map_err(|error| error.to_string())
}

fn guardrails(lifecycle: &LegacyLifecycle, input: &Input) -> Result<TracerChoice, String> {
    let bucket = if input.request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(OtelModelValue::Object(metadata)) = input.request_data.get(bucket) else {
        return Err(format!("request_data.{bucket} must be an object"));
    };
    let mut kwargs = input.kwargs.clone();
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        return Err("litellm_params must be an object".to_owned());
    };
    params.insert(
        "metadata".to_owned(),
        OtelModelValue::Object(metadata.clone()),
    );
    lifecycle
        .handle_success(&call_of(input, &kwargs))
        .map_err(|error| error.to_string())?;
    lifecycle
        .emit_request_guardrails(&input.request_data, None)
        .map_err(|error| error.to_string())?;
    lifecycle
        .tracer_choice(&kwargs)
        .map_err(|error| error.to_string())
}

fn observed(choice: &str, exporter: &InMemorySpanExporter) -> Result<Value, String> {
    let spans = exporter
        .get_finished_spans()
        .map_err(|error| error.to_string())?;
    let names: Vec<(SpanId, String)> = spans
        .iter()
        .map(|span| (span.span_context.span_id(), span.name.to_string()))
        .collect();
    let mut lines: Vec<(i64, i64, String, String, Value)> =
        spans.iter().map(|span| line(span, &names)).collect();
    lines.sort_by(|left, right| {
        (&left.0, &left.1, &left.2, &left.3).cmp(&(&right.0, &right.1, &right.2, &right.3))
    });
    let spans: Vec<Value> = lines.into_iter().map(|row| row.4).collect();
    Ok(json!({"choice": choice, "spans": spans}))
}

fn line(span: &SpanData, names: &[(SpanId, String)]) -> (i64, i64, String, String, Value) {
    let name = span.name.to_string();
    let parent = names
        .iter()
        .find(|(id, _)| *id == span.parent_span_id)
        .map_or_else(String::new, |(_, known)| known.clone());
    let start = system_ns(span.start_time);
    let end = system_ns(span.end_time);
    let value = json!({
        "name": name.clone(),
        "parent": parent.clone(),
        "kind": format!("{:?}", span.span_kind).to_uppercase(),
        "status": status_name(&span.status),
        "start": start.to_string(),
        "end": end.to_string(),
        "attributes": attributes_json(&span.attributes),
    });
    (start, end, name, parent, value)
}

fn system_ns(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |duration| {
        i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
    })
}

fn status_name(status: &Status) -> &'static str {
    match status {
        Status::Unset => "UNSET",
        Status::Ok => "OK",
        Status::Error { .. } => "ERROR",
    }
}

fn attributes_json(attributes: &[KeyValue]) -> BTreeMap<String, String> {
    attributes
        .iter()
        .map(|pair| (pair.key.as_str().to_owned(), value_text(&pair.value)))
        .collect()
}

fn value_text(value: &opentelemetry::Value) -> String {
    match value {
        opentelemetry::Value::Bool(flag) => flag.to_string(),
        opentelemetry::Value::I64(number) => number.to_string(),
        opentelemetry::Value::F64(number) => format!("{number:?}"),
        opentelemetry::Value::String(text) => json_text(text.as_str()),
        _ => String::new(),
    }
}

fn json_text(text: &str) -> String {
    if !(text.starts_with('{') || text.starts_with('[')) {
        return text.to_owned();
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return text.to_owned();
    };
    serde_json::to_string(&value).unwrap_or_else(|_| text.to_owned())
}
