use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_core::api::search_providers::linkup::{
    LinkupSearchConfig, build_request as build_linkup_request, decode_linkup_reply,
};
use betterlite_core::api::search_providers::searxng::{
    SearxngSearchConfig, build_request as build_searxng_request, decode_searxng_reply,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{SearchProvider, SearchQuery};
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

type Loaded<M> = Result<<M as SearchMapping>::Config, Box<HelperError>>;

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

struct Connection<'call> {
    environment: &'call BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<&'call str>,
}

trait SearchMapping {
    type Config: SearchProvider;

    fn load(connection: &Connection<'_>) -> Loaded<Self>;

    fn body(request: &Request) -> Result<<Self::Config as SearchProvider>::Body, Box<HelperError>>;

    fn reply(body: &str) -> Result<SearchResponse, Box<HelperError>>;

    fn answer(body: &str) -> HelperResult {
        let rendered = serde_json::to_value(Self::reply(body)?).map_err(failure)?;
        Ok(json!({"response": rendered}))
    }
}

struct LinkupMapping;

impl SearchMapping for LinkupMapping {
    type Config = LinkupSearchConfig;

    fn load(connection: &Connection<'_>) -> Loaded<Self> {
        let key = connection.api_key.clone().map(SecretString::from);
        LinkupSearchConfig::load(key, connection.api_base, connection.environment).map_err(failure)
    }

    fn body(request: &Request) -> Result<<Self::Config as SearchProvider>::Body, Box<HelperError>> {
        build_linkup_request(&request.query, &request.optional_params).map_err(failure)
    }

    fn reply(body: &str) -> Result<SearchResponse, Box<HelperError>> {
        decode_linkup_reply(body.as_bytes()).map_err(failure)
    }
}

struct SearxngMapping;

impl SearchMapping for SearxngMapping {
    type Config = SearxngSearchConfig;

    fn load(connection: &Connection<'_>) -> Loaded<Self> {
        let key = connection.api_key.clone().map(SecretString::from);
        SearxngSearchConfig::load(key, connection.api_base, connection.environment).map_err(failure)
    }

    fn body(request: &Request) -> Result<<Self::Config as SearchProvider>::Body, Box<HelperError>> {
        build_searxng_request(&request.query, &request.optional_params).map_err(failure)
    }

    fn reply(body: &str) -> Result<SearchResponse, Box<HelperError>> {
        decode_searxng_reply(body.as_bytes()).map_err(failure)
    }
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

fn bodies<M>(input: &BodyInput) -> HelperResult
where
    M: SearchMapping,
{
    input
        .requests
        .iter()
        .map(|request| serde_json::to_value(M::body(request)?).map_err(failure))
        .collect::<Result<Vec<_>, _>>()
        .map(|bodies| json!({"bodies": bodies}))
}

fn urls<M>(input: &UrlInput) -> HelperResult
where
    M: SearchMapping,
{
    let connection = Connection {
        environment: &input.environment,
        api_key: None,
        api_base: input.api_base.as_deref(),
    };
    let config = M::load(&connection)?;
    let mut rendered = Vec::new();
    for request in &input.requests {
        let body = M::body(request)?;
        let url = config
            .complete_url(
                input.api_base.as_deref(),
                &request.optional_params,
                Some(&body),
            )
            .map_err(failure)?;
        let (address, params) = split_url(&url);
        rendered.push(json!({"address": address, "params": params}));
    }
    Ok(json!({"urls": rendered}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::LinkupBody(request) => bodies::<LinkupMapping>(request),
            Input::LinkupReply(request) => LinkupMapping::answer(&request.body),
            Input::LinkupUrl(request) => urls::<LinkupMapping>(request),
            Input::LinkupEnvironment(request) => {
                let connection = Connection {
                    environment: &request.environment,
                    api_key: request.api_key.clone(),
                    api_base: request.api_base.as_deref(),
                };
                let headers = LinkupMapping::load(&connection)?.validate_environment(
                    request.headers.clone(),
                    None,
                    None,
                );
                Ok(json!({"headers": headers}))
            }
            Input::SearxngBody(request) => bodies::<SearxngMapping>(request),
            Input::SearxngReply(request) => SearxngMapping::answer(&request.body),
            Input::SearxngUrl(request) => urls::<SearxngMapping>(request),
            Input::SearxngEnvironment(request) => {
                let connection = Connection {
                    environment: &request.environment,
                    api_key: request.api_key.clone(),
                    api_base: request.api_base.as_deref(),
                };
                let headers = SearxngMapping::load(&connection)?.validate_environment(
                    request.headers.clone(),
                    None,
                    None,
                );
                Ok(json!({"headers": headers}))
            }
        }
    }
}
