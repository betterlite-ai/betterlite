use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::event::LitellmParams;
use super::header_metadata;

const OTEL_SPAN_KEY: &str = "litellm_parent_otel_span";

// LiteLLM keeps the span when there is no proxy_server_request, and the JSON encoder then fails on it.
#[must_use]
pub fn resolve(params: &LitellmParams) -> IndexMap<String, Json> {
    let mut metadata =
        header_metadata::merge(&params.metadata, params.proxy_server_request.as_ref());
    metadata.shift_remove(OTEL_SPAN_KEY);
    metadata
}

mod tests;
