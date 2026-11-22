use std::collections::BTreeMap;

use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::plumbing::context::w3c_propagation::inject_trace_context;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use opentelemetry::trace::SpanContext;

const TAG_HEADERS: [&str; 2] = ["tags", "x-litellm-tags"];
const TAGS_KEY: &str = "tags";

// _update_metadata_with_tags_in_header: both comma-separated tag headers extend metadata["tags"]
pub fn update_metadata_tags(headers: &HeaderMap, metadata: &mut Metadata) {
    let mut tags = Vec::new();
    for name in TAG_HEADERS {
        let found = headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty());
        if let Some(header_value) = found {
            tags.extend(
                header_value
                    .split(',')
                    .map(|tag| Json::Text(tag.trim().to_owned())),
            );
        }
    }
    if tags.is_empty() {
        return;
    }
    if let Some(Json::List(existing)) = metadata.get_mut(TAGS_KEY) {
        existing.extend(tags);
        return;
    }
    metadata.insert(TAGS_KEY.to_owned(), Json::List(tags));
}

// _with_trace_context: the OTel plumbing always links here, so only a missing span leaves headers unchanged
#[must_use]
pub fn with_trace_context(
    headers: &IndexMap<String, String>,
    parent_span: Option<&SpanContext>,
    trace: &RequestTraceContext,
) -> IndexMap<String, String> {
    let carrier: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    inject_trace_context(trace, &carrier, parent_span)
        .into_iter()
        .collect()
}
