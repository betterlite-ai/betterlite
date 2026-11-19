use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};

mod runs;
mod tests;

use betterlite_oracle_helper_api::error::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "case", rename_all = "snake_case")]
pub(crate) enum Input {
    Responses { runs: Vec<runs::ResponsesRun> },
    Passthrough { runs: Vec<runs::PassthroughRun> },
    Search { runs: Vec<runs::SearchRun> },
    VectorRetrieve { runs: Vec<runs::VectorRun> },
    RealtimeBridge { runs: Vec<runs::RealtimeRun> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let rows = match input {
            Input::Responses { runs } => runs
                .iter()
                .map(runs::responses_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::Passthrough { runs } => runs
                .iter()
                .map(runs::passthrough_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::Search { runs } => runs
                .iter()
                .map(runs::search_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::VectorRetrieve { runs } => runs
                .iter()
                .map(runs::vector_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            Input::RealtimeBridge { runs } => runs
                .iter()
                .map(runs::realtime_row)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
        };
        Ok(json!({ "runs": rows }))
    }
}

const ROW_KEYS: [&str; 8] = [
    "request",
    "endpoint_url",
    "streaming",
    "status",
    "reply",
    "response",
    "logged",
    "error",
];

pub(super) fn empty_row() -> Map<String, Value> {
    ROW_KEYS
        .into_iter()
        .map(|key| (key.to_owned(), Value::Null))
        .collect()
}

pub(super) fn error_row(kind: &str, status: Option<u16>) -> Value {
    let mut row = empty_row();
    row.insert(
        "error".to_owned(),
        json!({ "kind": kind, "status": status }),
    );
    Value::Object(row)
}

pub(super) fn request_row(
    method: &str,
    url: &str,
    headers: &[(String, Option<String>)],
    body: &[u8],
) -> Value {
    json!({
        "method": method,
        "url": url,
        "auth": auth_view(headers),
        "body": body_json(body),
    })
}

pub(super) fn auth_view(headers: &[(String, Option<String>)]) -> Value {
    let value = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .and_then(|(_, value)| value.as_deref());
    let Some(text) = value else {
        return Value::Null;
    };
    if let Some(token) = text.strip_prefix("Bearer ") {
        return json!({
            "kind": "bearer",
            "access_key": null,
            "region": null,
            "service": null,
            "token": token,
        });
    }
    let scope = text
        .split("Credential=")
        .nth(1)
        .unwrap_or_default()
        .split(',')
        .next()
        .unwrap_or_default();
    let parts: Vec<&str> = scope.split('/').collect();
    json!({
        "kind": "sigv4",
        "access_key": parts.first().copied(),
        "region": parts.get(2).copied(),
        "service": parts.get(3).copied(),
        "token": null,
    })
}

pub(super) fn body_json(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).into_owned()))
}
