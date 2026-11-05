use std::collections::BTreeMap;

#[must_use]
pub fn redact_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let shown = if name.eq_ignore_ascii_case("app-id") {
                "REDACTED"
            } else {
                value
            };
            (name.clone(), shown.to_owned())
        })
        .collect()
}

pub fn log_api_request(method: &str, url: &str, headers: &BTreeMap<String, String>, payload: &str) {
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
