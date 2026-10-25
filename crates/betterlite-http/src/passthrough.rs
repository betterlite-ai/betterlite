#![expect(
    clippy::disallowed_types,
    reason = "pass-through bodies intentionally retain provider JSON outside betterlite's model"
)]

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Map, Value};

const EXCLUDED_HEADERS: &[&str] = &[
    "transfer-encoding",
    "content-encoding",
    "content-length",
    "server",
    "date",
    "connection",
    "keep-alive",
];
#[must_use]
pub fn get_response_body(body: &[u8]) -> Option<Value> {
    let parsed = serde_json::from_slice(body);
    if body.is_empty() {
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
    let mut response_headers = filtered_headers(headers);
    if let Some(call_id) = litellm_call_id
        && let Ok(value) = HeaderValue::try_from(call_id)
    {
        response_headers.insert("x-litellm-call-id", value);
    }
    if let Some(custom) = custom_headers {
        response_headers.extend(
            custom
                .iter()
                .filter(|(name, _)| !is_excluded_header(name.as_str()))
                .map(|(name, value)| (name.clone(), value.clone())),
        );
    }
    response_headers
}

#[must_use]
pub fn streaming_response(headers: &HeaderMap) -> bool {
    let Some(content_type) = response_content_type(headers) else {
        return false;
    };
    content_type.contains("text/event-stream")
}

#[must_use]
pub fn buffer_response(status_code: u16, headers: &HeaderMap) -> bool {
    if status_code >= 400 {
        return true;
    }
    let media_type = response_content_type(headers)
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type.is_empty()
        || media_type == "application/json"
        || media_type.ends_with("+json")
        || media_type.starts_with("application/x-amz-json")
}

fn response_content_type(headers: &HeaderMap) -> Option<&str> {
    if headers.is_empty() {
        return None;
    }
    headers.get("content-type")?.to_str().ok()
}

#[must_use]
pub fn update_stream_param(
    request_body: &Map<String, Value>,
    stream: Option<bool>,
) -> Option<bool> {
    let request_stream = request_body.get("stream").and_then(Value::as_bool);
    request_stream.or(stream)
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
    let base = if base_target.ends_with('/') {
        base_target.to_owned()
    } else {
        format!("{base_target}/")
    };
    let without_leading_slash = subpath.strip_prefix('/').unwrap_or(subpath);
    let trailing_slash = without_leading_slash.ends_with('/');
    let (mut normalized, _) = normalize_path(&format!("/{without_leading_slash}"));
    if normalized == "/" {
        normalized.clear();
    } else {
        normalized = normalized.trim_start_matches('/').to_owned();
        if trailing_slash && !normalized.ends_with('/') {
            normalized.push('/');
        }
    }
    format!("{base}{normalized}")
}

#[must_use]
pub fn join_base_and_endpoint_path(base_url: &str, endpoint_path: &str) -> String {
    let base_path = path_from_url(base_url);
    let trailing_slash = endpoint_path.ends_with('/');
    if base_path.is_empty() || base_path == "/" {
        let (mut normalized, _) = normalize_path(&format!("/{endpoint_path}"));
        if trailing_slash && normalized != "/" && !normalized.ends_with('/') {
            normalized.push('/');
        }
        return normalized;
    }
    let clean_base = base_path.trim_end_matches('/');
    let endpoint = endpoint_path.trim_start_matches('/');
    let (mut combined, escaped) = normalize_path(&format!("{clean_base}/{endpoint}"));
    if escaped || (combined != clean_base && !combined.starts_with(&format!("{clean_base}/"))) {
        return format!("{clean_base}/");
    }
    if trailing_slash && !combined.ends_with('/') {
        combined.push('/');
    }
    combined
}

mod tests;

fn filtered_headers(headers: &HeaderMap) -> HeaderMap {
    if headers.is_empty() {
        return HeaderMap::new();
    }
    headers
        .iter()
        .filter(|(name, _)| !is_excluded_header(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn is_excluded_header(name: &str) -> bool {
    if EXCLUDED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
        return true;
    }
    false
}

fn path_from_url(base_url: &str) -> &str {
    let without_query = base_url.split_once('?').map_or(base_url, |(path, _)| path);
    if let Some((_, authority)) = without_query.split_once("//") {
        return path_after_authority(authority);
    }
    without_query
        .strip_prefix('/')
        .map_or("", |_| without_query)
}

fn path_after_authority(authority: &str) -> &str {
    let Some(index) = authority.find('/') else {
        return "";
    };
    let (_, path) = authority.split_at(index);
    path
}

fn normalize_path(path: &str) -> (String, bool) {
    let mut segments: Vec<&str> = Vec::new();
    let mut escaped = false;
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    escaped = true;
                }
            }
            value => segments.push(value),
        }
    }
    let normalized = format!("/{}", segments.join("/"));
    (normalized, escaped)
}
