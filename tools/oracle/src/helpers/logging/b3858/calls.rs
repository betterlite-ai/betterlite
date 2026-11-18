use std::collections::HashMap;

use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::legacy_lifecycle::context::instant_from_ns;
use betterlite_logging::otel::legacy_lifecycle::lifecycle::{
    LegacyLifecycle, LifecycleCall, LifecycleEvent,
};
use betterlite_logging::otel::legacy_policy::capture::LoggingFlags;
use betterlite_logging::otel::legacy_policy::span_time::{SpanTime, to_ns};
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::payloads::service::{ServiceLoggerPayload, ServiceType};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::service::error_attributes::record_error_attributes;
use betterlite_logging::otel::service::hooks::{
    ServiceSpanCall, async_service_failure_hook, async_service_success_hook,
};
use betterlite_logging::otel::service::management::{
    ManagementPayload, async_management_failure_hook, async_management_success_hook,
};
use betterlite_logging::otel::service::post_call::{
    PostCallFailure, ProxyCallKey, async_post_call_failure_hook, async_post_call_success_hook,
};
use betterlite_logging::otel::v2_callback::live_span::LiveSpan;
use betterlite_logging::otel::v2_callback::span_error::ErrorInformationConfig;
use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{Span as _, TraceContextExt as _};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde_json::json;

use super::{ExceptionInput, Input};

pub(super) struct CallContext<'call> {
    pub(super) lifecycle: &'call LegacyLifecycle,
    pub(super) input: &'call Input,
    pub(super) parent: Option<&'call LiveSpan>,
}

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Success,
    Failure,
}

fn service_type(name: &str) -> Result<ServiceType, String> {
    match name {
        "redis" => Ok(ServiceType::Redis),
        "postgres" => Ok(ServiceType::Postgres),
        "auth" => Ok(ServiceType::Auth),
        other => Err(format!("unknown service {other}")),
    }
}

fn exception_facts(input: &ExceptionInput) -> Result<ExceptionFacts, String> {
    let value = match input.kind.as_str() {
        "authentication" => json!({
            "class_name": "AuthenticationError",
            "message": format!("litellm.AuthenticationError: {}", input.message),
            "status_code": 401,
            "llm_provider": "openai",
        }),
        _ => json!({
            "class_name": "ValueError",
            "message": input.message.as_str(),
        }),
    };
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn error_config() -> Result<ErrorInformationConfig, String> {
    let normalizer = Normalizer::new().map_err(|error| error.to_string())?;
    Ok(ErrorInformationConfig {
        normalizer,
        redactor: None,
        log_client_error_tracebacks: false,
    })
}

fn management_payload(input: &Input) -> Result<ManagementPayload, String> {
    let exception = input.exception.as_ref().map(exception_facts).transpose()?;
    Ok(ManagementPayload {
        route: input.route.clone(),
        request_data: input.request_data.clone(),
        response: input.response.clone(),
        exception,
        start_time: SpanTime::Seconds(input.start_time),
        end_time: SpanTime::Seconds(input.end_time),
    })
}

pub(super) fn service_call(cx: &CallContext<'_>, outcome: Outcome) -> Result<(), String> {
    let payload = ServiceLoggerPayload {
        service: service_type(&cx.input.service)?,
        call_type: cx.input.call_type.clone(),
        caller: cx.input.caller.clone(),
        error: None,
    };
    let call = ServiceSpanCall {
        payload: &payload,
        parent: cx.parent,
        start_time: SpanTime::Seconds(cx.input.start_time),
        end_time: SpanTime::Seconds(cx.input.end_time),
        event_metadata: cx.input.event_metadata.as_ref(),
    };
    match outcome {
        Outcome::Success => async_service_success_hook(cx.lifecycle, &call, &cx.input.environment),
        Outcome::Failure => async_service_failure_hook(
            cx.lifecycle,
            &call,
            cx.input.error.as_deref(),
            &cx.input.environment,
        ),
    }
    Ok(())
}

pub(super) fn management_success_call(cx: &CallContext<'_>) -> Result<(), String> {
    let payload = management_payload(cx.input)?;
    async_management_success_hook(cx.lifecycle, &payload, cx.parent);
    Ok(())
}

pub(super) fn management_failure_call(cx: &CallContext<'_>) -> Result<(), String> {
    let payload = management_payload(cx.input)?;
    async_management_failure_hook(cx.lifecycle, &payload, cx.parent, &error_config()?);
    Ok(())
}

pub(super) fn success_call(cx: &CallContext<'_>) -> Result<(), String> {
    let mut details = cx.input.kwargs.clone();
    if let Some(live) = cx.parent {
        insert_carrier(&mut details, live);
    }
    async_post_call_success_hook(cx.lifecycle, &details, cx.parent)
        .map_err(|error| error.to_string())
}

pub(super) fn failure_call(cx: &CallContext<'_>) -> Result<(), String> {
    let facts = cx
        .input
        .exception
        .as_ref()
        .map(exception_facts)
        .transpose()?;
    let mut kwargs = cx.input.kwargs.clone();
    let mut request_data = cx.input.request_data.clone();
    if let Some(live) = cx.parent {
        insert_carrier(&mut kwargs, live);
    }
    if cx.input.after_failure_log {
        let call = LifecycleCall {
            kwargs: &kwargs,
            response: None,
            start_time: SpanTime::Seconds(cx.input.start_time),
            end_time: SpanTime::Seconds(cx.input.end_time),
            exception: None,
            flags: LoggingFlags {
                turn_off_message_logging: false,
                message_logging: true,
            },
        };
        cx.lifecycle
            .handle(&call, LifecycleEvent::Failure)
            .map_err(|error| error.to_string())?;
    }
    let team = cx.input.team.as_ref();
    let key = ProxyCallKey {
        parent_span: cx.parent,
        team_id: team.and_then(|team| team.id.as_deref()),
        team_alias: team.and_then(|team| team.alias.as_deref()),
    };
    let config = error_config()?;
    let call = PostCallFailure {
        request_data: &request_data,
        exception: facts.as_ref(),
        key: &key,
        traceback: cx.input.traceback.as_deref(),
        info: &config,
    };
    async_post_call_failure_hook(cx.lifecycle, &call).map_err(|error| error.to_string())
}

pub(super) fn error_attributes_call(cx: &CallContext<'_>) -> Result<(), String> {
    let facts = cx
        .input
        .exception
        .as_ref()
        .map(exception_facts)
        .transpose()?;
    record_error_attributes(
        cx.parent,
        facts.as_ref(),
        cx.input.status_code,
        &error_config()?,
    );
    Ok(())
}

// the parent span reaches LiteLLM through the propagated metadata, as a traceparent carrier
fn insert_carrier(kwargs: &mut MetadataMap, live: &LiveSpan) {
    if let Some(metadata) = carrier_bucket(kwargs, &["litellm_params", "metadata"]) {
        insert_parent(metadata, live);
    }
}

fn carrier_bucket<'map>(
    map: &'map mut MetadataMap,
    path: &[&str],
) -> Option<&'map mut MetadataMap> {
    let (bucket, rest) = path.split_first()?;
    let OtelModelValue::Object(inner) = map.get_mut(*bucket)? else {
        return None;
    };
    match rest {
        [] => Some(inner),
        _ => carrier_bucket(inner, rest),
    }
}

fn insert_parent(metadata: &mut MetadataMap, live: &LiveSpan) {
    let mut carrier: HashMap<String, String> = HashMap::new();
    TraceContextPropagator::new().inject_context(
        &Context::new().with_remote_span_context(live.context.clone()),
        &mut carrier,
    );
    let Some(value) = carrier.remove("traceparent") else {
        return;
    };
    metadata.insert(
        "litellm_parent_otel_span".to_owned(),
        OtelModelValue::String(value),
    );
}

pub(super) fn end_parent(input: &Input, parent: Option<&LiveSpan>) {
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
