use std::collections::BTreeMap;

use reqwest::header::HeaderMap;

#[must_use]
pub fn redact_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let shown = if name.as_str().eq_ignore_ascii_case("app-id") {
                "REDACTED".to_owned()
            } else {
                value.to_str().unwrap_or_default().to_owned()
            };
            (name.as_str().to_owned(), shown)
        })
        .collect()
}

pub fn log_api_request(method: &str, url: &str, headers: &HeaderMap, payload: &str) {
    tracing::debug!(
        method,
        url,
        headers = ?redact_headers(headers),
        payload,
        "Prompt Security request",
    );
}

pub fn log_api_response(url: &str, status_code: u16, payload: &str) {
    tracing::debug!(
        guardrail = "prompt_security",
        url,
        status_code,
        payload,
        "Prompt Security response",
    );
}
