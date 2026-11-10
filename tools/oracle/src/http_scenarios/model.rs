use std::collections::BTreeMap;

use betterlite_postgres_fixture::fixtures::http_scenarios::ScenarioSetup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::{BodyKind, Ignore, UpstreamAnswer};
use crate::http::Response;

use super::error::ScenarioError;

pub const POLLING_KEY_PREFIX: &str = "litellm:polling:response:";
pub const POLLING_TTL_SECONDS: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub min_steps: usize,
    pub max_steps: usize,
    pub max_input_bytes: usize,
    pub max_rows: usize,
    pub max_fixture_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            min_steps: 1,
            max_steps: 32,
            max_input_bytes: 16 * 1024 * 1024,
            max_rows: 256,
            max_fixture_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpScenario {
    pub(crate) name: String,
    pub(crate) config: String,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) setup: ScenarioSetup,
    pub(crate) steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) name: String,
    pub(crate) request: ScenarioRequest,
    #[serde(default)]
    pub(crate) upstream: Vec<UpstreamAnswer>,
    #[serde(default)]
    pub(crate) capture: Vec<Capture>,
    #[serde(default)]
    pub(crate) compare: StepCompare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScenarioRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: Value,
    #[serde(default)]
    pub(crate) body_kind: BodyKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capture {
    pub(crate) name: String,
    pub(crate) pointer: String,
    #[serde(default)]
    pub(crate) secret: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct StepCompare {
    #[serde(default)]
    pub(crate) headers: Vec<String>,
    #[serde(default)]
    pub(crate) headers_present: Vec<String>,
    #[serde(default)]
    pub(crate) upstream_headers: Vec<String>,
    #[serde(default)]
    pub(crate) ignore: Vec<Ignore>,
}

#[derive(Serialize)]
struct ResponseDocument<'input> {
    status: u16,
    headers: &'input BTreeMap<String, String>,
    body: &'input Value,
}

// the document a capture pointer resolves against: the wire response the client saw
pub(crate) fn response_document(response: &Response) -> Result<Value, ScenarioError> {
    serde_json::to_value(ResponseDocument {
        status: response.status,
        headers: &response.headers,
        body: &response.body,
    })
    .map_err(ScenarioError::Json)
}

pub(crate) struct CapturedValue {
    pub(crate) text: SecretString,
    pub(crate) secret: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Segment<'input> {
    Text(&'input str),
    Capture(&'input str),
}

pub(crate) fn segments(text: &str, index: usize) -> Result<Vec<Segment<'_>>, ScenarioError> {
    let mut segments = Vec::new();
    let mut remaining = text;
    while let Some((before, after)) = remaining.split_once("${capture.") {
        segments.push(Segment::Text(before));
        let Some((name, rest)) = after.split_once('}') else {
            return Err(ScenarioError::MalformedPlaceholder { index });
        };
        if name.is_empty() {
            return Err(ScenarioError::MalformedPlaceholder { index });
        }
        segments.push(Segment::Capture(name));
        remaining = rest;
    }
    segments.push(Segment::Text(remaining));
    Ok(segments)
}

pub(crate) struct Placeholders<'input> {
    pub(crate) index: usize,
    pub(crate) captures: &'input BTreeMap<String, CapturedValue>,
}

impl Placeholders<'_> {
    pub(crate) fn resolve(&self, text: &str) -> Result<String, ScenarioError> {
        let mut output = String::new();
        for segment in segments(text, self.index)? {
            self.append(&mut output, segment)?;
        }
        Ok(output)
    }

    fn append(&self, output: &mut String, segment: Segment<'_>) -> Result<(), ScenarioError> {
        match segment {
            Segment::Text(chunk) => output.push_str(chunk),
            Segment::Capture(name) => {
                let value =
                    self.captures
                        .get(name)
                        .ok_or_else(|| ScenarioError::UnresolvedCapture {
                            index: self.index,
                            name: name.to_owned(),
                        })?;
                output.push_str(value.text.expose_secret());
            }
        }
        Ok(())
    }
}

pub(crate) fn substitute_value(
    value: &Value,
    placeholders: &Placeholders<'_>,
) -> Result<Value, ScenarioError> {
    let mut output = value.clone();
    match &mut output {
        Value::String(text) => {
            let replaced = placeholders.resolve(text.as_str())?;
            *text = replaced;
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                *item = substitute_value(item, placeholders)?;
            }
        }
        Value::Object(fields) => {
            for item in fields.values_mut() {
                *item = substitute_value(item, placeholders)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(output)
}

pub(crate) fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}
