use std::collections::BTreeMap;
use std::sync::Mutex;

use indexmap::IndexMap;

use crate::otel::legacy_policy::dedupe::{EmittedSpans, HandlerIdentity, ScopeValue};
use crate::otel::legacy_policy::text_field;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

// LiteLLM keeps the markers on the shared kwargs dict; betterlite keeps them per request key instead.
const MAX_TRACKED_REQUESTS: usize = 10_000;

pub struct RequestDedupe {
    handler: HandlerIdentity,
    requests: Mutex<IndexMap<String, EmittedSpans>>,
}

impl RequestDedupe {
    #[must_use]
    pub fn new(class_name: String) -> Self {
        // one lifecycle owns its map, so the handler identity is the class and nothing else
        let handler = HandlerIdentity {
            class_name,
            instance: 0,
        };
        Self {
            handler,
            requests: Mutex::new(IndexMap::new()),
        }
    }

    // no request key means no shared dict either, so the call is its own first emission
    pub fn emit_once(&self, request_key: Option<&str>, scope: &[ScopeValue]) -> bool {
        let Some(key) = request_key else {
            return true;
        };
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !requests.contains_key(key) && requests.len() >= MAX_TRACKED_REQUESTS {
            _ = requests.shift_remove_index(0);
        }
        requests
            .entry(key.to_owned())
            .or_default()
            .emit_once(&self.handler, scope)
    }
}

// the request-local marker dict LiteLLM reads is keyed by the ids the proxy already shares across callbacks
#[must_use]
pub fn request_key(kwargs: &MetadataMap) -> Option<String> {
    let params = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object);
    let metadata = params
        .and_then(|fields| fields.get("metadata"))
        .and_then(OtelModelValue::as_object)
        .or_else(|| {
            params
                .and_then(|fields| fields.get("litellm_metadata"))
                .and_then(OtelModelValue::as_object)
        });
    metadata
        .and_then(|fields| {
            text_field(fields, "litellm_call_id").or_else(|| text_field(fields, "litellm_trace_id"))
        })
        .or_else(|| {
            kwargs
                .get("standard_logging_object")
                .and_then(OtelModelValue::as_object)
                .and_then(|payload| {
                    text_field(payload, "litellm_call_id").or_else(|| text_field(payload, "id"))
                })
        })
        .map(str::to_owned)
}

#[must_use]
pub fn scope_value(value: Option<&OtelModelValue>) -> ScopeValue {
    let Some(value) = value else {
        return ScopeValue::Null;
    };
    match value {
        OtelModelValue::Null(()) => ScopeValue::Null,
        OtelModelValue::Bool(flag) => ScopeValue::Bool(*flag),
        // Python's key equality reads 1 and 1.0 as one value, so every number freezes as a float
        OtelModelValue::Number(number) => ScopeValue::Float(number.as_f64().unwrap_or_default()),
        OtelModelValue::String(text) => ScopeValue::Text(text.clone()),
        OtelModelValue::Array(items) => {
            ScopeValue::List(items.iter().map(|item| scope_value(Some(item))).collect())
        }
        OtelModelValue::Object(fields) => ScopeValue::Map(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), scope_value(Some(item))))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}
