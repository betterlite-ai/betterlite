use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::legacy_lifecycle::context::instant_from_ns;
use betterlite_logging::otel::legacy_lifecycle::lifecycle::{
    DEFAULT_HANDLER_CLASS_NAME, LegacyLifecycle, LegacyLifecycleOptions, LifecycleCall,
    LifecycleEvent,
};
use betterlite_logging::otel::legacy_lifecycle::tracer::BaseDynamicOtel;
use betterlite_logging::otel::legacy_policy::capture::LoggingFlags;
use betterlite_logging::otel::legacy_policy::span_time::{SpanTime, to_ns};
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::events::SystemClock;
use betterlite_logging::otel::service::error_attributes::record_error_attributes_on_span;
use betterlite_logging::otel::service::hooks::{
    ServiceSpanCall, async_service_failure_hook, async_service_success_hook,
};
use betterlite_logging::otel::service::management::{
    ManagementEndpointLoggingPayload, async_management_endpoint_failure_hook,
    async_management_endpoint_success_hook,
};
use betterlite_logging::otel::service::post_call::{
    PostCallFailure, ProxyCallKey, async_post_call_failure_hook, async_post_call_success_hook,
};
use betterlite_logging::otel::v2_callback::live_span::{LiveSpan, RootSpans};
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use opentelemetry::trace::{Span as _, SpanContext};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use serde::Deserialize;

mod harness;
mod observe;
mod tests;
use harness::prepare;
use observe::observed;

pub(crate) struct Op;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Input {
    function: String,
    start_time: f64,
    end_time: f64,
    parent: Option<ParentInput>,
    service: String,
    call_type: String,
    caller: Option<String>,
    error: Option<String>,
    event_metadata: Option<MetadataMap>,
    kwargs: MetadataMap,
    request_data: MetadataMap,
    exception: Option<ExceptionInput>,
    team: Option<TeamInput>,
    after_failure_log: bool,
    route: String,
    response: Option<MetadataMap>,
    status_code: i64,
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ParentInput {
    name: String,
    start: Option<f64>,
    end: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ExceptionInput {
    kind: String,
    message: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TeamInput {
    id: Option<String>,
    alias: Option<String>,
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
                dynamic: Arc::new(BaseDynamicOtel),
                live: Arc::clone(&roots),
                metrics: None,
                events: None,
                failure_counters: Vec::new(),
                handler_class_name: DEFAULT_HANDLER_CLASS_NAME.to_owned(),
                use_request_span: false,
                clock: Box::new(SystemClock),
            },
        );
        let outcome = match input.function.as_str() {
            "service_success" | "service_failure" => {
                service_call(&lifecycle, input, parent.as_ref())
            }
            "post_call_failure" => failure_call(&lifecycle, input, parent.as_ref()),
            "post_call_success" => success_call(&lifecycle, input, parent.as_ref()),
            "management_success" | "management_failure" => {
                management_call(&lifecycle, input, parent.as_ref())
            }
            "error_attributes" => error_attributes_call(input, parent.as_ref()),
            other => Err(format!("unknown function {other}")),
        };
        outcome.or_value_error()?;
        end_parent(input, parent.as_ref());
        observed(&exporter).or_value_error()
    }
}

fn service_type(name: &str) -> Result<ServiceType, String> {
    match name {
        "redis" => Ok(ServiceType::Redis),
        "postgres" => Ok(ServiceType::Postgres),
        "auth" => Ok(ServiceType::Auth),
        other => Err(format!("unknown service {other}")),
    }
}

fn exception_facts(input: &ExceptionInput) -> ExceptionFacts {
    match input.kind.as_str() {
        "authentication" => ExceptionFacts {
            class_name: "AuthenticationError".to_owned(),
            message: Some(format!("litellm.AuthenticationError: {}", input.message)),
            status_code: Some(401),
            llm_provider: Some("openai".to_owned()),
            ..ExceptionFacts::default()
        },
        _ => ExceptionFacts {
            class_name: "ValueError".to_owned(),
            message: Some(input.message.clone()),
            ..ExceptionFacts::default()
        },
    }
}

fn error_config() -> Result<ErrorInformationConfig, String> {
    let normalizer = Normalizer::new().map_err(|error| error.to_string())?;
    Ok(ErrorInformationConfig {
        normalizer,
        redactor: None,
        log_client_error_tracebacks: false,
    })
}

fn service_call(
    lifecycle: &LegacyLifecycle,
    input: &Input,
    parent: Option<&LiveSpan>,
) -> Result<(), String> {
    let payload = ServiceLoggerPayload {
        service: service_type(&input.service)?,
        call_type: input.call_type.clone(),
        caller: input.caller.clone(),
        error: None,
    };
    let call = ServiceSpanCall {
        payload: &payload,
        parent,
        start_time: SpanTime::Seconds(input.start_time),
        end_time: SpanTime::Seconds(input.end_time),
        event_metadata: input.event_metadata.as_ref(),
    };
    if input.function == "service_success" {
        async_service_success_hook(lifecycle, &call, &input.environment);
        return Ok(());
    }
    async_service_failure_hook(lifecycle, &call, input.error.as_deref(), &input.environment);
    Ok(())
}

fn management_call(
    lifecycle: &LegacyLifecycle,
    input: &Input,
    parent: Option<&LiveSpan>,
) -> Result<(), String> {
    let payload = ManagementEndpointLoggingPayload {
        route: input.route.clone(),
        request_data: input.request_data.clone(),
        response: input.response.clone(),
        exception: input.exception.as_ref().map(exception_facts),
        start_time: SpanTime::Seconds(input.start_time),
        end_time: SpanTime::Seconds(input.end_time),
    };
    if input.function == "management_success" {
        async_management_endpoint_success_hook(lifecycle, &payload, parent);
        return Ok(());
    }
    let config = error_config()?;
    async_management_endpoint_failure_hook(lifecycle, &payload, parent, &config);
    Ok(())
}

fn success_call(
    lifecycle: &LegacyLifecycle,
    input: &Input,
    parent: Option<&LiveSpan>,
) -> Result<(), String> {
    let mut details = input.kwargs.clone();
    if let Some(live) = parent {
        insert_carrier(&mut details, live);
    }
    async_post_call_success_hook(lifecycle, &details, parent).map_err(|error| error.to_string())
}

fn failure_call(
    lifecycle: &LegacyLifecycle,
    input: &Input,
    parent: Option<&LiveSpan>,
) -> Result<(), String> {
    let facts = input.exception.as_ref().map(exception_facts);
    let mut kwargs = input.kwargs.clone();
    let mut request_data = input.request_data.clone();
    if let Some(live) = parent {
        insert_carrier(&mut kwargs, live);
        insert_bucket_carrier(&mut request_data, live);
    }
    if input.after_failure_log {
        let call = LifecycleCall {
            kwargs: &kwargs,
            response: None,
            start_time: SpanTime::Seconds(input.start_time),
            end_time: SpanTime::Seconds(input.end_time),
            exception: None,
            flags: LoggingFlags {
                turn_off_message_logging: false,
                message_logging: true,
            },
        };
        lifecycle
            .handle(&call, LifecycleEvent::Failure)
            .map_err(|error| error.to_string())?;
    }
    let key = ProxyCallKey {
        parent_span: parent,
        team_id: input.team.as_ref().and_then(|team| team.id.as_deref()),
        team_alias: input.team.as_ref().and_then(|team| team.alias.as_deref()),
    };
    let config = error_config()?;
    let call = PostCallFailure {
        request_data: &request_data,
        exception: facts.as_ref(),
        key: &key,
        traceback: None,
        info: &config,
    };
    async_post_call_failure_hook(lifecycle, &call).map_err(|error| error.to_string())
}

fn error_attributes_call(input: &Input, parent: Option<&LiveSpan>) -> Result<(), String> {
    let facts = input.exception.as_ref().map(exception_facts);
    let config = error_config()?;
    record_error_attributes_on_span(parent, facts.as_ref(), input.status_code, &config);
    Ok(())
}

// the parent span reaches LiteLLM through the propagated metadata, as a traceparent carrier
fn insert_carrier(kwargs: &mut MetadataMap, live: &LiveSpan) {
    let Some(OtelModelValue::Object(params)) = kwargs.get_mut("litellm_params") else {
        return;
    };
    let Some(OtelModelValue::Object(metadata)) = params.get_mut("metadata") else {
        return;
    };
    insert_parent(metadata, live);
}

// request_data keeps the proxy metadata at the top level, not under litellm_params
fn insert_bucket_carrier(request_data: &mut MetadataMap, live: &LiveSpan) {
    let bucket = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(OtelModelValue::Object(metadata)) = request_data.get_mut(bucket) else {
        return;
    };
    insert_parent(metadata, live);
}

fn insert_parent(metadata: &mut MetadataMap, live: &LiveSpan) {
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
