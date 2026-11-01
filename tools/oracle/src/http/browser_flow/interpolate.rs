use std::collections::BTreeMap;
use std::str;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use super::{BrowserFlowError, CapturedValue};
use crate::case::browser_flow::Step;
use crate::case::{BodyKind, RequestBody};
use crate::multipart_oracle::request::request_body;

// Case loading already rejected an undeclared capture or unknown encoding here.
pub(crate) fn resolve_text(
    index: usize,
    text: &str,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<String, BrowserFlowError> {
    const PREFIX: &str = "{{capture:";
    const SUFFIX: &str = "}}";
    let mut output = String::new();
    let mut remaining = text;
    while let Some((before, after_prefix)) = remaining.split_once(PREFIX) {
        output.push_str(before);
        let (body, after) =
            after_prefix
                .split_once(SUFFIX)
                .ok_or_else(|| BrowserFlowError::Interpolate {
                    index,
                    reason: "unterminated capture placeholder".to_owned(),
                })?;
        let mut parts = body.split(':');
        let name = parts.next().unwrap_or_default();
        let encoding = parts.next().unwrap_or_default();
        let stored = captures
            .get(name)
            .ok_or_else(|| BrowserFlowError::MissingCapture {
                index,
                name: name.to_owned(),
            })?;
        output.push_str(&encode(index, encoding, &stored.text)?);
        remaining = after;
    }
    output.push_str(remaining);
    Ok(output)
}

fn encode(index: usize, encoding: &str, text: &str) -> Result<String, BrowserFlowError> {
    match encoding {
        "raw" => Ok(text.to_owned()),
        "query" => Ok(utf8_percent_encode(text, NON_ALPHANUMERIC).to_string()),
        "form" => Ok(utf8_percent_encode(text, NON_ALPHANUMERIC)
            .to_string()
            .replace("%20", "+")),
        _ => Err(BrowserFlowError::Interpolate {
            index,
            reason: format!("unknown capture encoding \"{encoding}\""),
        }),
    }
}

// A JSON body is interpolated leaf by leaf so a captured value can never break JSON syntax.
pub(crate) fn resolve_body(
    index: usize,
    step: &Step,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<RequestBody, BrowserFlowError> {
    match step.request.body_kind {
        BodyKind::Json => {
            let value = resolve_value(index, &step.request.body, captures)?;
            request_body(&value, BodyKind::Json).map_err(|source| BrowserFlowError::Interpolate {
                index,
                reason: source.to_string(),
            })
        }
        BodyKind::Multipart => {
            request_body(&step.request.body, BodyKind::Multipart).map_err(|source| {
                BrowserFlowError::Interpolate {
                    index,
                    reason: source.to_string(),
                }
            })
        }
        BodyKind::Bytes => resolve_bytes_body(index, step, captures),
    }
}

fn resolve_bytes_body(
    index: usize,
    step: &Step,
    captures: &BTreeMap<String, CapturedValue>,
) -> Result<RequestBody, BrowserFlowError> {
    let decoded = request_body(&step.request.body, BodyKind::Bytes).map_err(|source| {
        BrowserFlowError::Interpolate {
            index,
            reason: source.to_string(),
        }
    })?;
    let RequestBody::Raw {
        content_type,
        bytes,
    } = decoded
    else {
        return Err(BrowserFlowError::Interpolate {
            index,
            reason: "bytes body did not decode to raw bytes".to_owned(),
        });
    };
    let text = str::from_utf8(&bytes).map_err(|_source| BrowserFlowError::Interpolate {
        index,
        reason: "fixture body is not valid UTF-8".to_owned(),
    })?;
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
