use betterlite_logging::otel::config::destination::{OtelDestination, SpanScope};
use betterlite_logging::otel::config::{
    CaptureMessageContent, ExporterOwner, ExporterSpec, Settings,
};
use serde_json::{Value, json};

pub(crate) fn settings_value(settings: &Settings) -> Value {
    let capture = match settings.capture_message_content {
        CaptureMessageContent::NoContent => "no_content",
        CaptureMessageContent::SpanOnly => "span_only",
        CaptureMessageContent::EventOnly => "event_only",
        CaptureMessageContent::SpanAndEvent => "span_and_event",
    };
    json!({
        "exporters": settings.exporters.iter().map(exporter_value).collect::<Vec<Value>>(),
        "mapper_names": &settings.mapper_names,
        "resource_attributes": &settings.resource_attributes,
        "service_name": &settings.service_name,
        "deployment_environment": &settings.deployment_environment,
        "capture_message_content": capture,
    })
}

fn exporter_value(spec: &ExporterSpec) -> Value {
    json!({
        "kind": &spec.kind,
        "endpoint": &spec.endpoint,
        "traces_endpoint": &spec.traces_endpoint,
        "headers": &spec.headers,
        "owner": spec.owner.map(owner_name),
        "options": &spec.options,
        "use_simple_processor": spec.use_simple_processor,
        "requires_headers": spec.requires_headers,
    })
}

const fn owner_name(owner: ExporterOwner) -> &'static str {
    match owner {
        ExporterOwner::ArizeAx => "arize",
        ExporterOwner::ArizePhoenix => "arize_phoenix",
        ExporterOwner::LangfuseOtel => "langfuse_otel",
        ExporterOwner::WeaveOtel => "weave_otel",
        ExporterOwner::Levo => "levo",
        ExporterOwner::Agentops => "agentops",
        ExporterOwner::Newrelic => "newrelic",
        ExporterOwner::Signoz => "signoz",
    }
}

pub(crate) fn destination_value(destination: &OtelDestination) -> Value {
    let fields = destination.as_ref();
    json!({
        "endpoint": &fields.endpoint,
        "headers": &fields.headers,
        "resource_attributes": &fields.resource_attributes,
        "callback_name": &fields.callback_name,
        "protocol": &fields.protocol,
        "span_scope": span_scope_name(fields.span_scope),
    })
}

const fn span_scope_name(scope: SpanScope) -> &'static str {
    match scope {
        SpanScope::Full => "full",
        SpanScope::LlmOnly => "llm_only",
    }
}
