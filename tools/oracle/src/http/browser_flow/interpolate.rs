use std::collections::BTreeMap;
use std::str::{self, Utf8Error};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::ExposeSecret;
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

// A JSON body is interpolated leaf by leaf so a captured value can never break JSON syntax.
pub(crate) fn resolve_body(
    index: usize,
    step: &Step,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<RequestBody, BrowserFlowError> {
    match step.request.body_kind {
        Bytes => resolve_bytes_body(index, step, captures),
        Json => {
            let resolved = resolve_value(index, &step.request.body, captures)?;
            request_body(&resolved, Json)
                .map_err(|source| BrowserFlowError::Multipart { index, source })
        }
        Multipart => request_body(&step.request.body, Multipart)
            .map_err(|source| BrowserFlowError::Multipart { index, source }),
    }
}

fn resolve_bytes_body(
    index: usize,
    step: &Step,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<RequestBody, BrowserFlowError> {
    let decoded = request_body(&step.request.body, Bytes)
        .map_err(|source| BrowserFlowError::Multipart { index, source })?;
    let RequestBody::Raw {
        content_type,
        bytes,
    } = decoded
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

fn resolve_value(
    index: usize,
    value: &Value,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<Value, BrowserFlowError> {
    match value {
        Value::String(text) => Ok(Value::String(resolve_text(index, text, captures)?)),
        Value::Array(items) => items
            .iter()
            .map(|item| resolve_value(index, item, captures))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, item)| Ok((key.clone(), resolve_value(index, item, captures)?)))
            .collect::<Result<serde_json::Map<_, _>, BrowserFlowError>>()
            .map(Value::Object),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
    }
}
