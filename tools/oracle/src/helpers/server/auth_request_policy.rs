use std::collections::BTreeMap;

use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;

mod answers;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    BytesToMb { values: Vec<i64> },
    Ip { requests: Vec<IpCase> },
    Classify { rows: Vec<ClassifyCase> },
    Mark { rows: Vec<MarkCase> },
    PreDb { requests: Vec<PreDbCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IpCase {
    forwarded_for: Option<String>,
    client_host: Option<String>,
    use_x_forwarded_for: bool,
    allowed_ips: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ErrorKind {
    Http,
    Proxy,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MarkerName {
    Absent,
    NotTrue,
    ExactTrue,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifyCase {
    kind: ErrorKind,
    code: Option<String>,
    status_code: u16,
    marker: MarkerName,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkCase {
    is_invalid_virtual_key: bool,
    code: String,
    message: String,
    openai_code: Option<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreDbCase {
    allowed_ips: Option<Vec<String>>,
    allowed_routes: Option<Vec<String>>,
    use_x_forwarded_for: bool,
    route: String,
    forwarded_for: Option<String>,
    client_host: Option<String>,
    content_length: Option<String>,
    max_request_size_mb: Option<f64>,
    body: JsonObject,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results: Vec<Value> = input.calls.iter().map(answers::answer).collect();
        Ok(Value::Array(results))
    }
}
