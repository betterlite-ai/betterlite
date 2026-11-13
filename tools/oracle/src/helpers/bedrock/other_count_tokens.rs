use std::collections::BTreeMap;

use betterlite_bedrock::count_tokens::detect_input_type;
use betterlite_bedrock::count_tokens::endpoint::{CountTokensEndpoint, count_tokens_endpoint};
use betterlite_bedrock::count_tokens::error::CountTokensError;
use betterlite_bedrock::count_tokens::response::transform_response;
use betterlite_bedrock::count_tokens::transform_request;
use betterlite_bedrock::count_tokens::validate::validate_count_tokens_request;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

const RUNTIME_ENDPOINT_VARIABLE: &str = "AWS_BEDROCK_RUNTIME_ENDPOINT";

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TypedBlockInvokeModel(SingleCall),
    UntypedMessagesConverse(MultiCall),
    ToolNameSchema(SingleCall),
    CountEndpointPath(EndpointCall),
    CountToAnthropicResult(ResponseCall),
    InvokeBodyPythonJsonSpacing(SingleCall),
    InvokeBodyDefaults(MultiCall),
    ConverseContentSystem(MultiCall),
    CountModelMissing(SingleCall),
    CountMessagesEmpty(SingleCall),
    CountMessageNotObject(SingleCall),
    CountMessageRoleMissing(SingleCall),
    CountMessageContentMissing(SingleCall),
    CountInvokeModelOnly(SingleCall),
    EmptyToolsOmission(MultiCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SingleCall {
    #[serde(default)]
    request: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MultiCall {
    #[serde(default)]
    requests: Vec<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointCall {
    #[serde(default)]
    cases: Vec<EndpointCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointCase {
    model: String,
    aws_region_name: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    aws_bedrock_runtime_endpoint: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    #[serde(default)]
    cases: Vec<IndexMap<String, Json>>,
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    CountTokens(#[from] CountTokensError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::TypedBlockInvokeModel(call)
            | Input::ToolNameSchema(call)
            | Input::InvokeBodyPythonJsonSpacing(call)
            | Input::CountModelMissing(call)
            | Input::CountMessagesEmpty(call)
            | Input::CountMessageNotObject(call)
            | Input::CountMessageRoleMissing(call)
            | Input::CountMessageContentMissing(call)
            | Input::CountInvokeModelOnly(call) => single_request(call),
            Input::UntypedMessagesConverse(call)
            | Input::InvokeBodyDefaults(call)
            | Input::ConverseContentSystem(call)
            | Input::EmptyToolsOmission(call) => multi_request(call),
            Input::CountEndpointPath(call) => Ok(endpoint(call)),
            Input::CountToAnthropicResult(call) => response(call),
        }
    }
}

fn single_request(call: &SingleCall) -> HelperResult {
    validate_count_tokens_request(&call.request).map_err(Failure::from)?;
    let request = transform_request(&call.request);
    Ok(serde_json::to_value(&request).map_err(Failure::from)?)
}

fn outcome(request: &IndexMap<String, Json>) -> Result<Value, Failure> {
    let input_type = detect_input_type(request);
    if validate_count_tokens_request(request).is_err() {
        return Ok(json!({"input_type": input_type, "validated": false}));
    }
    let transformed = transform_request(request);
    Ok(json!({
        "input_type": input_type,
        "validated": true,
        "request": serde_json::to_value(&transformed).map_err(Failure::from)?,
    }))
}

fn multi_request(call: &MultiCall) -> HelperResult {
    let mut outcomes = Vec::with_capacity(call.requests.len());
    for request in &call.requests {
        outcomes.push(outcome(request)?);
    }
    Ok(json!({"outcomes": outcomes}))
}

fn endpoint(call: &EndpointCall) -> Value {
    let endpoints = call
        .cases
        .iter()
        .map(|case| {
            count_tokens_endpoint(CountTokensEndpoint {
                model: &case.model,
                aws_region_name: &case.aws_region_name,
                api_base: case.api_base.as_deref(),
                aws_bedrock_runtime_endpoint: case.aws_bedrock_runtime_endpoint.as_deref(),
                environment_endpoint: case
                    .environment
                    .get(RUNTIME_ENDPOINT_VARIABLE)
                    .map(String::as_str),
            })
        })
        .collect::<Vec<String>>();
    json!({"endpoints": endpoints})
}

fn response(call: &ResponseCall) -> HelperResult {
    let mut results = Vec::new();
    for case in &call.cases {
        let result = transform_response(case);
        results.push(serde_json::to_value(result).map_err(Failure::from)?);
    }
    Ok(json!({"results": results}))
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::CountTokens(_) => "ValueError",
            Failure::Json(_) => "TypeError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
