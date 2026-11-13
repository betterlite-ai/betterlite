use std::collections::HashMap;

use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry::trace::{SpanContext, TraceContextExt as _};
use opentelemetry_sdk::propagation::TraceContextPropagator;

use crate::otel::model::metadata::wire::callback::{Callback, Params};
use crate::otel::model::metadata::wire::payload::Metadata;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::w3c_propagation::{TRACEPARENT, traceparent_from_header};

pub const PARENT_SPAN_KEY: &str = "litellm_parent_otel_span";

// what the kwargs name as the call's parent, with the active span the legacy hooks fall back to
#[derive(Debug, Clone, Default)]
pub struct ParentSpan {
    pub context: Option<Context>,
    pub span: Option<SpanContext>,
}

pub struct ParentSpanSources<'source> {
    pub metadata: Option<&'source Metadata>,
    pub params: Option<&'source Params>,
    pub span: Option<&'source OtelModelValue>,
}

// litellm.integrations.opentelemetry.OpenTelemetry._get_span_context
#[must_use]
pub fn span_context_from_kwargs(callback: &Callback) -> ParentSpan {
    let explicit = metadata_parent(&callback.litellm_params.metadata)
        .or_else(|| metadata_parent(&callback.litellm_params.litellm_metadata));
    if let Some(parent) = explicit {
        return ParentSpan {
            context: Some(Context::current().with_remote_span_context(parent)),
            span: None,
        };
    }
    if let Some(headers) = callback
        .litellm_params
        .proxy_server_request
        .headers
        .as_ref()
        && let Some(context) = traceparent_from_header(Some(headers))
    {
        return ParentSpan {
            context: Some(context),
            span: None,
        };
    }
    let current = Context::current();
    let active = current.span().span_context().clone();
    if active.is_valid() {
        return ParentSpan {
            context: Some(current),
            span: Some(active),
        };
    }
    ParentSpan::default()
}

// litellm.litellm_core_utils.core_helpers._get_parent_otel_span_from_kwargs
#[must_use]
pub fn parent_otel_span_from_kwargs(sources: &ParentSpanSources<'_>) -> Option<SpanContext> {
    if let Some(metadata) = sources.metadata
        && let Some(value) = metadata.extra.get(PARENT_SPAN_KEY)
    {
        return span_context_from_value(value);
    }
    if let Some(params) = sources.params
        && let Some(value) = params.metadata.extra.get(PARENT_SPAN_KEY)
    {
        return span_context_from_value(value);
    }
    sources.span.and_then(span_context_from_value)
}

fn metadata_parent(metadata: &Metadata) -> Option<SpanContext> {
    metadata
        .extra
        .get(PARENT_SPAN_KEY)
        .and_then(span_context_from_value)
}

// betterlite's typed carrier for the span LiteLLM keeps as a live object in the metadata dict
#[must_use]
pub fn span_context_from_value(input: &OtelModelValue) -> Option<SpanContext> {
    let carrier: HashMap<String, String> = match input {
        OtelModelValue::String(text) => HashMap::from([(TRACEPARENT.to_owned(), text.clone())]),
        OtelModelValue::Object(fields) => fields
            .iter()
            .filter_map(|(key, value)| {
                value
                    .as_str()
                    .map(|text| (key.to_lowercase(), text.to_owned()))
            })
            .collect(),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::Array(_) => return None,
    };
    let context = TraceContextPropagator::new().extract_with_context(&Context::new(), &carrier);
    let span_context = context.span().span_context().clone();
    span_context.is_valid().then_some(span_context)
}

#[path = "parent_span/tests.rs"]
mod tests;
