#![expect(
    clippy::disallowed_types,
    reason = "pass-through bodies intentionally retain provider JSON outside betterlite's model"
)]

use reqwest::Url;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Map, Value};

const EXCLUDED_HEADERS: [&str; 7] = [
    "transfer-encoding",
    "content-encoding",
    "content-length",
    "server",
    "date",
    "connection",
    "keep-alive",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseHandling {
    Stream,
    Buffer,
    Relay,
}

#[must_use]
pub fn get_response_body(body: &[u8]) -> Option<Value> {
    let parsed = serde_json::from_slice(body);
    if parsed.is_err() {
        return None;
    }
    parsed.ok()
}

#[must_use]
pub fn get_response_headers(
    headers: &HeaderMap,
    litellm_call_id: Option<&str>,
    custom_headers: Option<&HeaderMap>,
) -> HeaderMap {
    let mut response_headers = HeaderMap::new();
    for (name, value) in headers {
        if !EXCLUDED_HEADERS.contains(&name.as_str()) {
            response_headers.append(name, value.clone());
        }
    }
    if let Some(call_id) = litellm_call_id.filter(|call_id| !call_id.is_empty())
        && let Ok(value) = HeaderValue::try_from(call_id)
    {
        response_headers.insert("x-litellm-call-id", value);
    }
    if let Some(resolved_headers) = custom_headers {
        for (name, value) in resolved_headers {
            if !EXCLUDED_HEADERS.contains(&name.as_str()) {
                response_headers.insert(name, value.clone());
            }
        }
    }
    response_headers
}

#[must_use]
pub fn response_handling(status_code: u16, headers: &HeaderMap) -> ResponseHandling {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if content_type.contains("text/event-stream") {
        ResponseHandling::Stream
    } else if status_code >= 400
        || media_type.is_empty()
        || media_type == "application/json"
        || media_type.ends_with("+json")
        || media_type.starts_with("application/x-amz-json")
    {
        ResponseHandling::Buffer
    } else {
        ResponseHandling::Relay
    }
}

#[must_use]
pub fn streaming_response(headers: &HeaderMap) -> bool {
    let Some(content_type) = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    content_type.contains("text/event-stream")
}

#[must_use]
pub fn buffer_response(status_code: u16, headers: &HeaderMap) -> bool {
    if status_code >= 400 {
        return true;
    }
    let handling = response_handling(status_code, headers);
    handling == ResponseHandling::Buffer
}

#[must_use]
pub fn update_stream_param(
    request_body: &Map<String, Value>,
    stream: Option<bool>,
) -> Option<bool> {
    let stream_value = request_body.get("stream");
    stream_value.map_or(stream, Value::as_bool)
}

#[must_use]
pub fn construct_url_with_subpath(
    base_target: &str,
    subpath: &str,
    include_subpath: bool,
) -> String {
    if !include_subpath || subpath.is_empty() {
        return base_target.to_owned();
    }
    let separator = if base_target.ends_with('/') { "" } else { "/" };
    let normalized = normalize_path(&format!("/{subpath}"));
    let mut safe_subpath = normalized.trim_start_matches('/').to_owned();
    if subpath.ends_with('/') && !safe_subpath.is_empty() {
        safe_subpath.push('/');
    }
    format!("{base_target}{separator}{safe_subpath}")
}

#[must_use]
pub fn join_base_and_endpoint_path(base_url: &str, endpoint_path: &str) -> String {
    let parsed_url = Url::parse(base_url).ok();
    let base_path = parsed_url
        .as_ref()
        .map_or("", Url::path)
        .trim_end_matches('/');
    let endpoint = endpoint_path.trim_start_matches('/');
    let mut combined = normalize_path(&format!("{base_path}/{endpoint}"));
    if !base_path.is_empty()
        && combined != base_path
        && !combined.starts_with(&format!("{base_path}/"))
    {
        return format!("{base_path}/");
    }
    if endpoint_path.ends_with('/') && combined != "/" {
        combined.push('/');
    }
    combined
}

mod tests;

// Same result as posixpath.normpath for a path that starts with a slash.
fn normalize_path(path: &str) -> String {
    let root = if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else {
        "/"
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    format!("{root}{}", segments.join("/"))
}
