use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_logging::otel::model::metadata::wire::callback::{Params, ProxyRequest};
use betterlite_logging::otel::model::metadata::wire::payload::Metadata;
use betterlite_logging::otel::plumbing::context::parent_span::{
    PARENT_SPAN_KEY, ParentSpanSources, parent_otel_span_from_kwargs,
};

use super::json::key_value_json;

pub const PARENT_OTEL_SPAN_KWARG: &str = "parent_otel_span";
const METADATA_KEY: &str = "metadata";
const LITELLM_PARAMS_KEY: &str = "litellm_params";

// LiteLLM stores the span it found back into the kwargs under parent_otel_span; betterlite writes the W3C traceparent it can carry
pub fn add_parent_otel_span(kwargs: &mut KeyObject) {
    let metadata = metadata_with_parent(kwargs.get(METADATA_KEY));
    let params = params_with_parent(kwargs.get(LITELLM_PARAMS_KEY));
    let span = kwargs.get(PARENT_SPAN_KEY).and_then(|value| {
        let encoded = serde_json::to_value(key_value_json(value)).ok()?;
        serde_json::from_value(encoded).ok()
    });
    let found = parent_otel_span_from_kwargs(&ParentSpanSources {
        metadata: metadata.as_ref(),
        params: params.as_ref(),
        span: span.as_ref(),
    });
    let stored = found.map_or(KeyValue::Null, |context| {
        KeyValue::String(format!(
            "00-{}-{}-{:02x}",
            hex::encode(context.trace_id().to_bytes()),
            hex::encode(context.span_id().to_bytes()),
            context.trace_flags().to_u8()
        ))
    });
    kwargs.insert(PARENT_OTEL_SPAN_KWARG.to_owned(), stored);
}

fn metadata_with_parent(value: Option<&KeyValue>) -> Option<Metadata> {
    let KeyValue::Object(fields) = value? else {
        return None;
    };
    let field = fields.get(PARENT_SPAN_KEY)?;
    let encoded = serde_json::to_value(key_value_json(field)).ok()?;
    let span = serde_json::from_value(encoded).ok()?;
    let mut metadata = Metadata::default();
    metadata.extra.insert(PARENT_SPAN_KEY.to_owned(), span);
    Some(metadata)
}

fn params_with_parent(value: Option<&KeyValue>) -> Option<Params> {
    let KeyValue::Object(fields) = value? else {
        return None;
    };
    let metadata = metadata_with_parent(fields.get(METADATA_KEY))?;
    Some(Params {
        metadata,
        litellm_metadata: Metadata::default(),
        litellm_session_id: None,
        proxy_server_request: ProxyRequest { headers: None },
    })
}
