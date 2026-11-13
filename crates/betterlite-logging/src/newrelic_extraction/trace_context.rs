use betterlite_models::decisions::Json;

use crate::newrelic_extraction::ids::UuidSource;
use crate::newrelic_extraction::sources::EventSources;

const TRACEPARENT: &str = "traceparent";

pub fn resolve_trace_id(sources: &EventSources<'_>, ids: &dyn UuidSource) -> String {
    if let Some(trace_id) = trace_from_headers(sources) {
        return trace_id;
    }
    if let Some(payload) = sources
        .standard_logging_object
        .filter(|payload| !payload.trace_id.is_empty())
    {
        return payload.trace_id.clone();
    }
    ids.next_id().simple().to_string()
}

fn trace_from_headers(sources: &EventSources<'_>) -> Option<String> {
    let headers = sources.kwargs.headers?;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case(TRACEPARENT) {
            let Json::Text(traceparent) = value else {
                return None;
            };
            return trace_from_traceparent(traceparent);
        }
    }
    None
}

fn trace_from_traceparent(traceparent: &str) -> Option<String> {
    let mut parts = traceparent.split('-');
    let (Some(_version), Some(trace_id), Some(_span_id), Some(_flags), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    if trace_id.is_empty() {
        return None;
    }
    Some(trace_id.to_owned())
}
