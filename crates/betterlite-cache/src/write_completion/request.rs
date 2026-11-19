use betterlite_logging::otel::model::metadata::wire::callback::{Params, ProxyRequest};
use betterlite_logging::otel::model::metadata::wire::payload::Metadata;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::context::parent_span::{
    ParentSpanSources, parent_otel_span_from_kwargs,
};

use crate::cache_key::request::{CacheKeyRequest, KeyParam, LiteLlmParams, RequestCacheControl};
use crate::cache_key::value::{KeyObject, KeyValue};
use crate::envelope::request::EnvelopeRequest;
use crate::handler::HandlerError;
use crate::handler::kwargs::{
    convert_args_to_kwargs, key_request, request_cache_key, request_control,
};
use crate::handler::state::OriginalFunction;
use crate::types::control::DynamicCacheControl;

pub const PARENT_OTEL_SPAN_KWARG: &str = "parent_otel_span";
const PARENT_SPAN_KEY: &str = "litellm_parent_otel_span";
const METADATA_KWARG: &str = "metadata";
const LITELLM_PARAMS_KWARG: &str = "litellm_params";

// async_set_cache's copies kwargs before the positional arguments overwrite names of the original function's parameters
pub(crate) fn merged_request_kwargs(
    kwargs: &KeyObject,
    original_function: &OriginalFunction,
    args: &[KeyValue],
) -> KeyObject {
    let mut merged = kwargs.clone();
    merged.extend(convert_args_to_kwargs(&original_function.parameters, args));
    merged
}

// LiteLLM stores the span it found back into the kwargs under parent_otel_span; betterlite writes the W3C traceparent it can carry
pub fn add_parent_otel_span(kwargs: &mut KeyObject) {
    let metadata = metadata_with_parent_span(kwargs.get(METADATA_KWARG));
    let params = params_with_parent_span(kwargs.get(LITELLM_PARAMS_KWARG));
    let span = kwargs.get(PARENT_SPAN_KEY).map(otel_value);
    let parent = parent_otel_span_from_kwargs(&ParentSpanSources {
        metadata: metadata.as_ref(),
        params: params.as_ref(),
        span: span.as_ref(),
    });
    let stored = parent.map_or_else(
        || KeyValue::Null,
        |found| {
            KeyValue::String(format!(
                "00-{}-{}-{:02x}",
                hex::encode(found.trace_id().to_bytes()),
                hex::encode(found.span_id().to_bytes()),
                found.trace_flags().to_u8()
            ))
        },
    );
    kwargs.insert(PARENT_OTEL_SPAN_KWARG.to_owned(), stored);
}

// Only the key B3847's lookup reads is carried into the typed metadata; every other field stays with the logger that owns it.
fn metadata_with_parent_span(value: Option<&KeyValue>) -> Option<Metadata> {
    let KeyValue::Object(fields) = value? else {
        return None;
    };
    let span = fields.get(PARENT_SPAN_KEY)?;
    let mut metadata = Metadata::default();
    metadata
        .extra
        .insert(PARENT_SPAN_KEY.to_owned(), otel_value(span));
    Some(metadata)
}

fn params_with_parent_span(value: Option<&KeyValue>) -> Option<Params> {
    let KeyValue::Object(fields) = value? else {
        return None;
    };
    let metadata = metadata_with_parent_span(fields.get(METADATA_KWARG))?;
    Some(Params {
        metadata,
        litellm_metadata: Metadata::default(),
        litellm_session_id: None,
        proxy_server_request: ProxyRequest { headers: None },
    })
}

fn otel_value(value: &KeyValue) -> OtelModelValue {
    match value {
        KeyValue::Null => OtelModelValue::Null(()),
        KeyValue::Bool(flag) => OtelModelValue::Bool(*flag),
        KeyValue::Number(number) => OtelModelValue::Number(number.clone()),
        KeyValue::String(text) => OtelModelValue::String(text.clone()),
        KeyValue::Array(items) => OtelModelValue::Array(items.iter().map(otel_value).collect()),
        KeyValue::Object(fields) => OtelModelValue::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), otel_value(entry)))
                .collect(),
        ),
    }
}

// One write's cache call, rebuilt for every attempt: kwargs merged, span resolved, key derivation input fixed.
#[derive(Clone)]
pub(crate) struct CacheWriteRequest {
    cache_key: Option<String>,
    params: Vec<KeyParam>,
    metadata: Option<KeyObject>,
    litellm_metadata: Option<KeyObject>,
    litellm_params: Option<LiteLlmParams>,
    cache_control: Option<RequestCacheControl>,
    control: DynamicCacheControl,
}

impl CacheWriteRequest {
    pub(crate) fn prepare(kwargs: &KeyObject) -> Result<Self, HandlerError> {
        let key = key_request(kwargs)?;
        Ok(Self {
            cache_key: request_cache_key(kwargs)?,
            params: key.params,
            metadata: key.metadata,
            litellm_metadata: key.litellm_metadata,
            litellm_params: key.litellm_params,
            cache_control: key.cache_control,
            control: request_control(kwargs)?,
        })
    }

    pub(crate) fn envelope(&self) -> EnvelopeRequest {
        EnvelopeRequest {
            cache_key: self.cache_key.clone(),
            key: CacheKeyRequest {
                params: self.params.clone(),
                metadata: self.metadata.clone(),
                litellm_metadata: self.litellm_metadata.clone(),
                litellm_params: self.litellm_params.clone(),
                cache_control: self.cache_control.clone(),
            },
            control: self.control.clone(),
        }
    }
}
