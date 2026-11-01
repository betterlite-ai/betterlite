use std::collections::BTreeSet;
use std::path::Path;
use std::str::{self, Utf8Error};

use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::{BodyKind, Case, Compare, Request, RequestBody, validate_ignores};
use crate::OracleError;
use crate::multipart_oracle::request::request_body;

pub(crate) const MAX_STEPS: usize = 64;
pub(crate) const MAX_CAPTURES: usize = 128;
pub(crate) const MAX_CAPTURE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) request: Request,
    pub(crate) compare: Compare,
    #[serde(default)]
    pub(crate) captures: Vec<Capture>,
    #[serde(default)]
    pub(crate) form: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capture {
    pub(crate) name: String,
    pub(crate) source: CaptureSource,
    #[serde(default)]
    pub(crate) secret: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CaptureSource {
    Body { pointer: String },
    Header { name: String },
    Cookie { name: String },
    LocationQuery { name: String },
    Form { control: String },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Encoding {
    Raw,
    Query,
    Form,
}

#[derive(Debug, Clone)]
pub(crate) struct PlaceholderRef {
    pub(crate) name: String,
    pub(crate) encoding: Encoding,
}

#[derive(Debug, Error)]
pub enum FormCaptureError {
    #[error("form response is not valid UTF-8: {0}")]
    Utf8(#[source] Utf8Error),
    #[error("response has {count} form elements, not exactly one")]
    FormCount { count: usize },
    #[error("cannot parse selector \"{0}\"")]
    Selector(String),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FormProjection {
    pub method: String,
    pub action: Option<String>,
    pub controls: Vec<(String, Option<String>)>,
}

pub(crate) fn parse_form_projection(text: &str) -> Result<FormProjection, FormCaptureError> {
    let document = Html::parse_document(text);
    let form_selector =
        Selector::parse("form").map_err(|_source| FormCaptureError::Selector("form".to_owned()))?;
    let forms = document.select(&form_selector).collect::<Vec<_>>();
    let [form] = forms.as_slice() else {
        return Err(FormCaptureError::FormCount { count: forms.len() });
    };
    let method = form
        .value()
        .attr("method")
        .map_or_else(|| "get".to_owned(), str::to_ascii_lowercase);
    let action = form.value().attr("action").map(str::to_owned);
    let control_selector = Selector::parse("input[name], button[name]")
        .map_err(|_source| FormCaptureError::Selector("input[name], button[name]".to_owned()))?;
    let controls = form
        .select(&control_selector)
        .map(|element| {
            let name = element.value().attr("name").unwrap_or_default().to_owned();
            let value = element.value().attr("value").map(str::to_owned);
            (name, value)
        })
        .collect();
    Ok(FormProjection {
        method,
        action,
        controls,
    })
}

// Serde defaults the unused side of request/compare or steps, so emptiness stands in for absence.
pub(crate) fn validate_request_or_steps(path: &Path, case: &Case) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if let Some(steps) = &case.steps {
        if *case.request != Request::default() || *case.compare != Compare::default() {
            return Err(invalid("a case cannot set both request/compare and steps"));
        }
        return validate_steps(path, steps);
    }
    if case.request.path.is_empty() {
        return Err(invalid("request.path cannot be empty"));
    }
    request_body(&case.request.body, case.request.body_kind).map_err(OracleError::Multipart)?;
    validate_ignores(path, &case.compare.ignore)
}

pub(crate) fn validate_steps(path: &Path, steps: &[Step]) -> Result<(), OracleError> {
    let invalid = |message: String| OracleError::CaseInvalid {
        path: path.to_owned(),
        message,
    };
    if steps.is_empty() {
        return Err(invalid("steps cannot be empty".to_owned()));
    }
    if steps.len() > MAX_STEPS {
        return Err(invalid(format!("steps cannot exceed {MAX_STEPS}")));
    }
    let mut known_names = BTreeSet::new();
    let mut total_captures = 0_usize;
    for (index, step) in steps.iter().enumerate() {
        validate_step(path, index, step, &known_names)?;
        for capture in &step.captures {
            if capture.name.is_empty() {
                return Err(invalid(format!(
                    "step {index} capture name cannot be empty"
                )));
            }
            if !known_names.insert(capture.name.clone()) {
                return Err(invalid(format!(
                    "step {index} capture name \"{}\" is already declared",
                    capture.name
                )));
            }
            total_captures = total_captures.saturating_add(1);
        }
    }
    if total_captures > MAX_CAPTURES {
        return Err(invalid(format!("captures cannot exceed {MAX_CAPTURES}")));
    }
    Ok(())
}

fn validate_step(
    path: &Path,
    index: usize,
    step: &Step,
    known_names: &BTreeSet<String>,
) -> Result<(), OracleError> {
    let invalid = |message: String| OracleError::CaseInvalid {
        path: path.to_owned(),
        message,
    };
    if step.request.path.is_empty() {
        return Err(invalid(format!(
            "step {index} request.path cannot be empty"
        )));
    }
    let decoded =
        request_body(&step.request.body, step.request.body_kind).map_err(OracleError::Multipart)?;
    validate_ignores(path, &step.compare.ignore)?;
    let refs = step_placeholder_refs(step, &decoded)
        .map_err(|message| invalid(format!("step {index}: {message}")))?;
    for reference in &refs {
        if !known_names.contains(&reference.name) {
            return Err(invalid(format!(
                "step {index} references undeclared capture \"{}\"",
                reference.name
            )));
        }
    }
    Ok(())
}

fn step_placeholder_refs(
    step: &Step,
    decoded_body: &RequestBody,
) -> Result<Vec<PlaceholderRef>, String> {
    let mut refs = find_placeholders(&step.request.path)?;
    if refs
        .iter()
        .any(|reference| reference.encoding != Encoding::Query)
    {
        return Err("path captures must use query encoding".to_owned());
    }
    for value in step.request.headers.values() {
        refs.extend(find_placeholders(value)?);
    }
    collect_value_placeholders(&step.request.body, &mut refs)?;
    if step.request.body_kind == BodyKind::Bytes
        && let RequestBody::Raw { bytes, .. } = decoded_body
    {
        let text = str::from_utf8(bytes)
            .map_err(|_source| "fixture body is not valid UTF-8".to_owned())?;
        let fixture_refs = find_placeholders(text)?;
        if fixture_refs
            .iter()
            .any(|reference| reference.encoding == Encoding::Raw)
        {
            return Err("a url-encoded or XML fixture body cannot use raw encoding".to_owned());
        }
        refs.extend(fixture_refs);
    }
    Ok(refs)
}

fn collect_value_placeholders(value: &Value, refs: &mut Vec<PlaceholderRef>) -> Result<(), String> {
    match value {
        Value::String(text) => {
            refs.extend(find_placeholders(text)?);
            Ok(())
        }
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| collect_value_placeholders(item, refs)),
        Value::Object(fields) => fields
            .values()
            .try_for_each(|item| collect_value_placeholders(item, refs)),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

pub(crate) fn find_placeholders(text: &str) -> Result<Vec<PlaceholderRef>, String> {
    const PREFIX: &str = "{{capture:";
    const SUFFIX: &str = "}}";
    let mut found = Vec::new();
    let mut remaining = text;
    while let Some((_, after_prefix)) = remaining.split_once(PREFIX) {
        let Some((body, after)) = after_prefix.split_once(SUFFIX) else {
            return Err("unterminated capture placeholder".to_owned());
        };
        let mut parts = body.split(':');
        let name = parts.next().unwrap_or_default();
        let encoding = parts.next().unwrap_or_default();
        if parts.next().is_some() || name.is_empty() || encoding.is_empty() {
            return Err(format!("malformed capture placeholder \"{body}\""));
        }
        if !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        }) {
            return Err(format!(
                "capture name \"{name}\" has unsupported characters"
            ));
        }
        let parsed = match encoding {
            "raw" => Encoding::Raw,
            "query" => Encoding::Query,
            "form" => Encoding::Form,
            _ => return Err(format!("unknown capture encoding \"{encoding}\"")),
        };
        found.push(PlaceholderRef {
            name: name.to_owned(),
            encoding: parsed,
        });
        remaining = after;
    }
    Ok(found)
}
