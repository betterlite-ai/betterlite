mod tests;

pub const EVENT_STREAM_MEDIA_TYPE: &str = "application/vnd.amazon.eventstream";

const EVENT_STREAM_CONTENT_FRAGMENT: &str = "vnd.amazon.eventstream";

// Which handler owns the endpoint: this module, or the generic passthrough handler B3573 builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routed {
    Handled,
    Generic,
}

#[must_use]
pub fn is_converse_endpoint(endpoint: &str) -> bool {
    matches!(
        endpoint.trim_end_matches('/').split('/').next_back(),
        Some("converse" | "converse-stream")
    )
}

#[must_use]
pub fn is_event_stream_content_type(content_type: &str) -> bool {
    content_type.contains(
        // LiteLLM matches the fragment anywhere in the header, not the whole media type
        EVENT_STREAM_CONTENT_FRAGMENT,
    )
}

#[must_use]
// ast-grep-ignore: no-long-names, no-one-line-functions
pub fn event_stream_endpoint_is_de_anonymizable(endpoint: &str) -> bool {
    is_converse_endpoint(endpoint)
}
