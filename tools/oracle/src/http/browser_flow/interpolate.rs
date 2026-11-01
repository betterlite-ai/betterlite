use std::collections::BTreeMap;
use std::str::{self, Utf8Error};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::ExposeSecret as _;
use serde_json::Value;
use url::form_urlencoded;

use super::{BrowserFlowError, CapturedValue};
use crate::case::BodyKind::{Bytes, Json, Multipart};
use crate::case::RequestBody;
use crate::case::browser_flow::{
    Encoding, PLACEHOLDER_PREFIX, PLACEHOLDER_SUFFIX, Step, find_placeholders,
};
use crate::multipart_oracle::request::request_body;

// Case loading already validated each placeholder, so find_placeholders is read here, not reparsed.
pub(crate) fn resolve_text(
    index: usize,
    text: &str,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<String, BrowserFlowError> {
    let placeholders = find_placeholders(text)
        .map_err(|source| BrowserFlowError::Placeholder { index, source })?;
    let mut output = String::new();
    let mut remaining = text;
    for placeholder in &placeholders {
        let (before, after_prefix) = remaining
            .split_once(PLACEHOLDER_PREFIX)
            .ok_or(BrowserFlowError::Interpolate { index })?;
        output.push_str(before);
        let (_, after) = after_prefix
            .split_once(PLACEHOLDER_SUFFIX)
            .ok_or(BrowserFlowError::Interpolate { index })?;
        let stored =
            captures
                .get(&placeholder.name)
                .ok_or_else(|| BrowserFlowError::MissingCapture {
                    index,
                    name: placeholder.name.clone(),
                })?;
        output.push_str(&encode(placeholder.encoding, stored.text.expose_secret()));
        remaining = after;
    }
    output.push_str(remaining);
    Ok(output)
}

fn encode(encoding: Encoding, text: &str) -> String {
    match encoding {
        Encoding::Raw => text.to_owned(),
        Encoding::Query => utf8_percent_encode(text, NON_ALPHANUMERIC).to_string(),
        Encoding::Form => form_urlencoded::byte_serialize(text.as_bytes()).collect(),
    }
}

// A JSON body is interpolated leaf by leaf so a captured value can never break JSON syntax; a bytes body is decoded to UTF-8 first.
pub(crate) fn resolve_body(
    index: usize,
    step: &Step,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<RequestBody, BrowserFlowError> {
    let multipart = |source| BrowserFlowError::Multipart { index, source };
    match step.request.body_kind {
        Bytes => {
            let RequestBody::Raw {
                content_type,
                bytes,
            } = request_body(&step.request.body, Bytes).map_err(multipart)?
            else {
                return Err(BrowserFlowError::BytesBody { index });
            };
            let text = str::from_utf8(&bytes)
                .map_err(|source: Utf8Error| BrowserFlowError::BodyUtf8 { index, source })?;
            let interpolated = resolve_text(index, text, captures)?;
            Ok(RequestBody::Raw {
                content_type,
                bytes: interpolated.into_bytes(),
            })
        }
        Json => {
            let resolved = resolve_value(index, &step.request.body, captures)?;
            request_body(&resolved, Json).map_err(multipart)
        }
        Multipart => request_body(&step.request.body, Multipart).map_err(multipart),
    }
}

fn resolve_value(
    index: usize,
    value: &Value,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<Value, BrowserFlowError> {
    match value {
        Value::String(text) => Ok(Value::String(resolve_text(index, text, captures)?)),
        Value::Array(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                resolved.push(resolve_value(index, item, captures)?);
            }
            Ok(Value::Array(resolved))
        }
        Value::Object(fields) => {
            let mut resolved = serde_json::Map::new();
            for (key, item) in fields {
                resolved.insert(key.clone(), resolve_value(index, item, captures)?);
            }
            Ok(Value::Object(resolved))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
    }
}
