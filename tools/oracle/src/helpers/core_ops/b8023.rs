use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_core::api::search_providers::linkup::{
    LinkupSearchConfig, build_request as build_linkup_request, decode_linkup_reply,
};
use betterlite_core::api::search_providers::searxng::{
    SearxngSearchConfig, build_request as build_searxng_request, decode_searxng_reply,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{SearchProvider as _, SearchQuery};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
pub(crate) struct Request {
    query: SearchQuery,
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BodyInput {
    requests: Vec<Request>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnvironmentInput {
    headers: BTreeMap<String, String>,
    environment: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlInput {
    environment: BTreeMap<String, String>,
    api_base: Option<String>,
    requests: Vec<Request>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplyInput {
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    LinkupBody(BodyInput),
    LinkupEnvironment(EnvironmentInput),
    LinkupReply(ReplyInput),
    LinkupUrl(UrlInput),
    SearxngBody(BodyInput),
    SearxngEnvironment(EnvironmentInput),
    SearxngReply(ReplyInput),
    SearxngUrl(UrlInput),
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "SearchProviderError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn split_url(url: &str) -> (&str, Vec<&str>) {
    match url.split_once('?') {
        Some((address, query)) => {
            let mut params: Vec<&str> = query.split('&').collect();
            params.sort_unstable();
            (address, params)
        }
        None => (url, Vec::new()),
    }
}

fn linkup_body(input: &BodyInput) -> HelperResult {
    let mut bodies = Vec::new();
    for request in &input.requests {
        let body =
            build_linkup_request(&request.query, &request.optional_params).map_err(failure)?;
        bodies.push(serde_json::to_value(body).map_err(failure)?);
    }
    Ok(json!({"bodies": bodies}))
}

fn searxng_body(input: &BodyInput) -> HelperResult {
    let mut bodies = Vec::new();
    for request in &input.requests {
        let body =
            build_searxng_request(&request.query, &request.optional_params).map_err(failure)?;
        bodies.push(serde_json::to_value(body).map_err(failure)?);
    }
    Ok(json!({"bodies": bodies}))
}

fn linkup_url(input: &UrlInput) -> HelperResult {
    let config = LinkupSearchConfig::load(None, input.api_base.as_deref(), &input.environment)
        .map_err(failure)?;
    let mut urls = Vec::new();
    for request in &input.requests {
        let body =
            build_linkup_request(&request.query, &request.optional_params).map_err(failure)?;
        let url = config
            .complete_url(
                input.api_base.as_deref(),
                &request.optional_params,
                Some(&body),
            )
            .map_err(failure)?;
        let (address, params) = split_url(&url);
        urls.push(json!({"address": address, "params": params}));
    }
    Ok(json!({"urls": urls}))
}

fn searxng_url(input: &UrlInput) -> HelperResult {
    let config = SearxngSearchConfig::load(None, input.api_base.as_deref(), &input.environment)
        .map_err(failure)?;
    let mut urls = Vec::new();
    for request in &input.requests {
        let body =
            build_searxng_request(&request.query, &request.optional_params).map_err(failure)?;
        let url = config
            .complete_url(
                input.api_base.as_deref(),
                &request.optional_params,
                Some(&body),
            )
            .map_err(failure)?;
        let (address, params) = split_url(&url);
        urls.push(json!({"address": address, "params": params}));
    }
    Ok(json!({"urls": urls}))
}

fn linkup_environment(input: &EnvironmentInput) -> HelperResult {
    let config = LinkupSearchConfig::load(
        input.api_key.clone().map(SecretString::from),
        input.api_base.as_deref(),
        &input.environment,
    )
    .map_err(failure)?;
    let headers = config.validate_environment(input.headers.clone(), None, None);
    Ok(json!({"headers": headers}))
}

fn searxng_environment(input: &EnvironmentInput) -> HelperResult {
    let config = SearxngSearchConfig::load(
        input.api_key.clone().map(SecretString::from),
        input.api_base.as_deref(),
        &input.environment,
    )
    .map_err(failure)?;
    let headers = config.validate_environment(input.headers.clone(), None, None);
    Ok(json!({"headers": headers}))
}

fn linkup_reply(input: &ReplyInput) -> HelperResult {
    decode_linkup_reply(input.body.as_bytes())
        .map_err(failure)
        .and_then(|response| serde_json::to_value(response).map_err(failure))
        .map(|response| json!({"response": response}))
}

fn searxng_reply(input: &ReplyInput) -> HelperResult {
    decode_searxng_reply(input.body.as_bytes())
        .map_err(failure)
        .and_then(|response| serde_json::to_value(response).map_err(failure))
        .map(|response| json!({"response": response}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::LinkupBody(request) => linkup_body(request),
            Input::LinkupEnvironment(request) => linkup_environment(request),
            Input::LinkupReply(request) => linkup_reply(request),
            Input::LinkupUrl(request) => linkup_url(request),
            Input::SearxngBody(request) => searxng_body(request),
            Input::SearxngEnvironment(request) => searxng_environment(request),
            Input::SearxngReply(request) => searxng_reply(request),
            Input::SearxngUrl(request) => searxng_url(request),
        }
    }
}
