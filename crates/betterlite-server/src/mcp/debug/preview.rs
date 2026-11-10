pub mod cookie;
pub mod secrets;
mod tests;
pub mod text;
pub mod upstream;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::decode::decode_mcp_stored_contract;
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::REDACTED;
use encoding_rs::UTF_8;
use http::{HeaderMap, HeaderValue};

use self::secrets::sensitive_field;
use self::text::safe_text;
use self::upstream::{BodyState, UpstreamFailure, UpstreamResponse};

pub const BODY_CAPTURE_BYTES: usize = 0x4000;
pub const CAPTURE_TIMEOUT_SECONDS: u64 = 1;
pub(super) const UNSTRUCTURED_BODY: &str = "(omitted: unstructured body)";
pub(super) const CREDENTIALS_UNAVAILABLE: &str = "(omitted: request credentials unavailable)";
pub(super) const CAPTURE_FAILED: &str = "(unavailable: error body read failed)";
const EMPTY_BODY: &str = "(empty)";
const BODY_TOO_LARGE: &str = "(omitted: body exceeds capture limit)";
const STREAMED_BODY: &str = "(streamed, not captured)";
const UNREAD_BODY: &str = "(not read)";
const SAFE_HEADER_NAMES: [&str; 3] = ["accept", "content-length", "content-type"];
pub(super) const FORM_URLENCODED: &str = "application/x-www-form-urlencoded";

#[must_use]
pub fn preview(raw: &[u8], content_type: &str, secrets: &[String], redactor: &Redactor) -> String {
    if raw.is_empty() {
        return EMPTY_BODY.to_owned();
    }
    if raw.len() > BODY_CAPTURE_BYTES {
        return BODY_TOO_LARGE.to_owned();
    }
    let Ok(body) = decode_mcp_stored_contract(raw) else {
        return unstructured_preview(raw, content_type, secrets);
    };
    let redacted = redact_object(body);
    match &redacted {
        JsonValue::Object(_) | JsonValue::Array(_) => {
            let Ok(serialized) = serde_json::to_string(&redacted) else {
                return UNSTRUCTURED_BODY.to_owned();
            };
            safe_text(&redactor.redact_string(&secrets::mask_known_values(&serialized, secrets)))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            UNSTRUCTURED_BODY.to_owned()
        }
    }
}

fn redact_object(body: JsonValue) -> JsonValue {
    match body {
        JsonValue::Object(fields) => JsonValue::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    if sensitive_field(&key) {
                        (key, JsonValue::String(REDACTED.to_owned()))
                    } else {
                        (key, redact_object(value))
                    }
                })
                .collect(),
        ),
        JsonValue::Array(items) => JsonValue::Array(items.into_iter().map(redact_object).collect()),
        value @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)) => value,
    }
}

fn unstructured_preview(raw: &[u8], content_type: &str, secrets: &[String]) -> String {
    let text = UTF_8.decode_without_bom_handling(raw).0;
    if content_type_base(content_type) != FORM_URLENCODED
        || !text.contains('=')
        || text.contains(['<', '>', '\n', '\r'])
    {
        return UNSTRUCTURED_BODY.to_owned();
    }
    let fields: Vec<String> = secrets::parse_qsl(&text, true)
        .into_iter()
        .map(|(key, field_value)| {
            let redacted = if sensitive_field(&key) {
                REDACTED.to_owned()
            } else {
                field_value
            };
            format!(
                "{}={}",
                secrets::plus_encode(&key),
                secrets::plus_encode(&redacted)
            )
        })
        .collect();
    safe_text(&secrets::mask_known_values(&fields.join("&"), secrets))
}

#[must_use]
pub fn masked_headers(headers: &HeaderMap) -> String {
    let parts: Vec<String> = headers
        .iter()
        .filter(|(name, _value)| SAFE_HEADER_NAMES.contains(&name.as_str()))
        .map(|(name, value)| format!("{}={}", name.as_str(), header_text(value)))
        .collect();
    safe_text(&parts.join(", "))
}

#[must_use]
pub fn content_type(headers: &HeaderMap) -> String {
    headers
        .iter()
        .find(|(name, _value)| name.as_str() == "content-type")
        .map_or_else(String::new, |(_name, value)| header_text(value))
}

pub(super) fn content_type_base(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase()
}

pub(super) fn header_text(value: &HeaderValue) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}

#[must_use]
pub fn describe_upstream_response(response: &UpstreamResponse, redactor: &Redactor) -> String {
    let Some(request) = response.request.as_ref() else {
        return format!("HTTP {} | request unavailable", response.status.as_u16());
    };
    let secrets = secrets::request_secret_values(request);
    let request_body = match &request.body {
        BodyState::NotRead => STREAMED_BODY.to_owned(),
        BodyState::Read(raw) => preview(
            raw,
            &content_type(&request.headers),
            secrets.as_deref().unwrap_or_default(),
            redactor,
        ),
    };
    let response_body = secrets.map_or_else(
        || CREDENTIALS_UNAVAILABLE.to_owned(),
        |found| {
            response
                .captured
                .clone()
                .unwrap_or_else(|| match &response.body {
                    BodyState::NotRead => UNREAD_BODY.to_owned(),
                    BodyState::Read(raw) => {
                        preview(raw, &content_type(&response.headers), &found, redactor)
                    }
                })
        },
    );
    format!(
        "{} {} -> HTTP {} | request headers: {} | request body: {request_body} | response body: {response_body}",
        safe_text(&request.method),
        text::safe_upstream_url(&request.url),
        response.status.as_u16(),
        masked_headers(&request.headers),
    )
}

#[must_use]
pub fn describe_upstream_http_failure(
    failure: &dyn UpstreamFailure,
    redactor: &Redactor,
) -> Option<String> {
    let mut descriptions: Vec<String> = Vec::new();
    let mut current = Some(failure);
    let mut visited = 0_usize;
    while visited < 16 {
        let Some(node) = current else {
            break;
        };
        visited = visited.saturating_add(1);
        if let Some(response) = node.response() {
            descriptions.push(describe_upstream_response(response, redactor));
        }
        current = node.cause();
    }
    if descriptions.is_empty() {
        None
    } else {
        Some(descriptions.join(" | "))
    }
}
