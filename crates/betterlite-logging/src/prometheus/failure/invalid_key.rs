use super::exception::LabelException;
use super::status_code::extract_status_code;

const AUTH_ERROR_PATTERNS: [&str; 5] = [
    "virtual key expected",
    "expected to start with 'sk-'",
    "authentication error",
    "invalid api key",
    "api key not valid",
];

#[must_use]
pub fn is_invalid_api_key_request(
    status_code: Option<i64>,
    exception: Option<&LabelException>,
) -> bool {
    if status_code == Some(401) {
        return true;
    }
    let Some(failure) = exception else {
        return false;
    };
    let message = failure
        .message
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    AUTH_ERROR_PATTERNS
        .iter()
        .copied()
        .any(|pattern| message.contains(pattern))
}

#[must_use]
// ast-grep-ignore: no-long-names - B8561 calls this filter by the LiteLLM name
pub fn should_skip_metrics_for_invalid_key(
    enum_status_code: Option<&str>,
    exception: Option<&LabelException>,
    kwargs_exception: Option<&LabelException>,
) -> bool {
    let status_code = extract_status_code(enum_status_code, exception, kwargs_exception);
    let checked = exception.or(kwargs_exception);
    if !is_invalid_api_key_request(status_code, checked) {
        return false;
    }
    tracing::debug!(
        status_code = ?status_code,
        exception = ?checked.map(|failure| failure.class_name.as_str()),
        "Skipping Prometheus metrics for invalid API key request"
    );
    true
}
