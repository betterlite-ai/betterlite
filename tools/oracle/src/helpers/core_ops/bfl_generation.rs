use std::collections::BTreeMap;

use betterlite_core::api::bfl::common::{BflError, check_bfl_poll_url, decode_bfl_reply};
use betterlite_core::api::bfl::generation::{
    SUPPORTED_OPENAI_PARAMS, apply_image_response, complete_url, map_openai_params, request_body,
    validate_environment,
};
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::reply::ImageDatum;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct MappingRequest {
    params: IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MappingParams {
    model: String,
    prompt: String,
    drop_params: bool,
    requests: Vec<MappingRequest>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CredentialRequest {
    api_key: Option<String>,
    api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CredentialParams {
    model: String,
    environment: BTreeMap<String, String>,
    requests: Vec<CredentialRequest>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AppendReply {
    existing: Vec<ImageDatum>,
    body: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AppendParams {
    now: i64,
    replies: Vec<AppendReply>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PollParams {
    urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Mapping(MappingParams),
    Credentials(CredentialParams),
    AppendResults(AppendParams),
    PollHost(PollParams),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Mapping(params) => mapping(params),
            Input::Credentials(params) => credentials(params),
            Input::AppendResults(params) => append_results(params),
            Input::PollHost(params) => poll_host(params),
        }
    }
}

fn failure(error: BflError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BflError".to_owned(),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn mapping(params: &MappingParams) -> HelperResult {
    let mut results = Vec::new();
    for request in &params.requests {
        let mapped = map_openai_params(
            &request.params,
            request.optional_params.clone(),
            &params.model,
            params.drop_params,
        )
        .map_err(failure)?;
        let body = request_body(&params.prompt, &mapped);
        results.push(json!({"params": mapped, "body": body}));
    }
    Ok(json!({"supported": SUPPORTED_OPENAI_PARAMS, "results": results}))
}

fn credentials(params: &CredentialParams) -> HelperResult {
    let environment = &params.environment;
    let mut outcomes = Vec::new();
    for request in &params.requests {
        let key = request.api_key.clone().map(SecretString::from);
        let prepared =
            validate_environment(HeaderMap::new(), key.as_ref(), environment).map_err(failure)?;
        let mut headers = Map::new();
        for (name, value) in &prepared {
            let text = value.to_str().map_err(|error| invalid(error.to_string()))?;
            headers.insert(name.as_str().to_owned(), json!(text));
        }
        let url = complete_url(request.api_base.as_deref(), &params.model, environment)
            .map_err(failure)?;
        outcomes.push(json!({"headers": headers, "url": url}));
    }
    Ok(json!({"outcomes": outcomes}))
}

fn append_results(params: &AppendParams) -> HelperResult {
    let mut responses = Vec::new();
    for reply in &params.replies {
        let bytes = serde_json::to_vec(&reply.body).map_err(|error| invalid(error.to_string()))?;
        let decoded = decode_bfl_reply(&bytes).map_err(failure)?;
        let response =
            apply_image_response(&decoded, reply.existing.clone(), params.now).map_err(failure)?;
        responses.push(response);
    }
    Ok(json!({"responses": responses}))
}

fn poll_host(params: &PollParams) -> HelperResult {
    let mut outcomes = Vec::new();
    for url in &params.urls {
        let accepted = check_bfl_poll_url(url).is_ok();
        outcomes.push(json!({"url": url, "accepted": accepted}));
    }
    Ok(json!({"urls": outcomes}))
}
