use std::time::Duration;

use axum::http::header::InvalidHeaderName;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use betterlite_error::envelope::{is_bare_rate_limit_header, is_unsafe_header_name};
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatError;

// LiteLLM's OPENAI_RESPONSE_HEADERS, repeated without the llm_provider- prefix on a success
const RATE_LIMIT_HEADERS: [&str; 6] = [
    "x-ratelimit-remaining-requests",
    "x-ratelimit-remaining-tokens",
    "x-ratelimit-limit-requests",
    "x-ratelimit-limit-tokens",
    "x-ratelimit-reset-requests",
    "x-ratelimit-reset-tokens",
];

pub(super) fn success_headers(
    upstream: &HeaderMap,
    deployment: &ChatDeployment,
    call_id: &str,
    upstream_time: Duration,
    total: Duration,
) -> HeaderMap {
    let mut headers = provider_headers(upstream, RATE_LIMIT_HEADERS.len());
    let overhead = total.saturating_sub(upstream_time);
    let values = [
        ("x-litellm-call-id", call_id.to_owned()),
        ("x-litellm-model-id", deployment.model_id.clone()),
        ("x-litellm-model-name", deployment.litellm_model.clone()),
        ("x-litellm-model-group", deployment.model_group.clone()),
        ("x-litellm-model-api-base", deployment.api_base.clone()),
        ("x-litellm-version", env!("CARGO_PKG_VERSION").to_owned()),
        (
            "x-litellm-response-duration-ms",
            milliseconds(upstream_time),
        ),
        ("x-litellm-overhead-duration-ms", milliseconds(overhead)),
        ("x-litellm-callback-duration-ms", "0.0".to_owned()),
        ("x-litellm-attempted-retries", "0".to_owned()),
        ("x-litellm-attempted-fallbacks", "0".to_owned()),
    ];
    insert_all(&mut headers, values);
    headers
}

pub(super) fn stream_headers(
    upstream: &HeaderMap,
    deployment: &ChatDeployment,
    call_id: &str,
    upstream_time: Duration,
    total: Duration,
) -> HeaderMap {
    let mut headers = success_headers(upstream, deployment, call_id, upstream_time, total);
    headers.insert(
        "content-type",
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert("cache-control", HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    headers
}

pub(super) fn failure_headers(
    error: &ChatError,
    call_id: &str,
    deployment: Option<&ChatDeployment>,
) -> HeaderMap {
    let upstream = match error {
        ChatError::Provider { response } => Some(&response.headers),
        ChatError::UnknownModel { .. }
        | ChatError::Request { .. }
        | ChatError::Streaming
        | ChatError::UnsupportedProvider { .. }
        | ChatError::MissingApiKey
        | ChatError::Timeout
        | ChatError::Encode { .. }
        | ChatError::Decode { .. }
        | ChatError::TooLarge
        | ChatError::Http { .. } => None,
    };
    let mut headers = upstream.map_or_else(HeaderMap::new, forwardable_failure_headers);
    if let Ok(value) = HeaderValue::from_str(call_id) {
        headers.insert("x-litellm-call-id", value);
    }
    // LiteLLM reports the timeout only once the router has picked a deployment
    if let Some(picked) = deployment
        && let Ok(value) = HeaderValue::try_from(python_float(picked.timeout_seconds))
    {
        headers.insert("x-litellm-timeout", value);
    }
    headers
}

fn provider_prefixed_name(name: &HeaderName) -> Result<HeaderName, InvalidHeaderName> {
    if name.as_str().contains("llm_provider") {
        Ok(name.clone())
    } else {
        HeaderName::try_from(format!("llm_provider-{name}"))
    }
}

// LiteLLM forwards every provider header as llm_provider-{name} and repeats OpenAI's rate limit headers as they are
fn provider_headers(upstream: &HeaderMap, rate_limit_count: usize) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in upstream {
        if let Ok(forwarded_name) = provider_prefixed_name(name) {
            headers.append(forwarded_name, value.clone());
        }
    }
    for name in RATE_LIMIT_HEADERS.iter().take(rate_limit_count) {
        if let Some(value) = upstream.get(*name) {
            headers.insert(*name, value.clone());
        }
    }
    headers
}

// get_response_headers() projected through llm_provider- then UNSAFE_PROXY_RESPONSE_HEADERS, kept on HeaderMap so a non-UTF-8 value never needs a lossy to_str
fn forwardable_failure_headers(upstream: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in upstream {
        if is_bare_rate_limit_header(name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
        match provider_prefixed_name(name) {
            Ok(forwarded_name) if is_unsafe_header_name(forwarded_name.as_str()) => {}
            Ok(forwarded_name) => {
                headers.append(forwarded_name, value.clone());
            }
            Err(error) => {
                tracing::warn!(header = %name, %error, "upstream header name cannot be forwarded");
            }
        }
    }
    headers
}

fn insert_all<Values>(headers: &mut HeaderMap, values: Values)
where
    Values: IntoIterator<Item = (&'static str, String)>,
{
    for (name, text) in values {
        match HeaderValue::try_from(text) {
            Ok(value) => {
                headers.insert(name, value);
            }
            Err(error) => {
                tracing::warn!(header = name, %error, "response header value is not valid");
            }
        }
    }
}

fn milliseconds(duration: Duration) -> String {
    let micros = duration
        .subsec_micros()
        .checked_rem(1000)
        .unwrap_or_default();
    format!("{}.{micros:03}", duration.as_millis())
}

// Python writes a float that has no fraction as 6000.0
fn python_float(value: f64) -> String {
    let text = value.to_string();
    if text.contains(['.', 'e', 'i', 'N']) {
        text
    } else {
        format!("{text}.0")
    }
}

mod tests;
