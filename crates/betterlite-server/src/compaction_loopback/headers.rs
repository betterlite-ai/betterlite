use axum::http::HeaderMap;

mod tests;

const REMOVED_HEADERS: [&str; 5] = [
    "content-length",
    "x-litellm-call-id",
    "x-litellm-num-retries",
    "x-litellm-timeout",
    "x-litellm-stream-timeout",
];

const REDACTION_OPT_OUT: &str = "litellm-disable-message-redaction";

#[must_use]
pub fn child_headers(parent: &HeaderMap, logging_disabled: bool) -> HeaderMap {
    let mut child = HeaderMap::with_capacity(parent.len());
    for (name, value) in parent {
        let removed = REMOVED_HEADERS
            .iter()
            .any(|candidate| name.as_str().eq_ignore_ascii_case(candidate));
        let opted_out = logging_disabled && name.as_str().eq_ignore_ascii_case(REDACTION_OPT_OUT);
        if !removed && !opted_out {
            child.append(name, value.clone());
        }
    }
    child
}
