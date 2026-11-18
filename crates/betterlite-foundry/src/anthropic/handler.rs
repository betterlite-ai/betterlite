use std::error::Error as StdError;

use betterlite_anthropic::anthropic_http_stream::completion::ParameterError;
use betterlite_anthropic::anthropic_http_stream::transport::StreamTransport;
use betterlite_anthropic::chat_call::ChatDeps;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::config::{ChatConfig, TransformRequest};
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::ChatTransport;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use futures_util::future::LocalBoxFuture;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::CUSTOM_LLM_PROVIDER;
use super::environment::{EnvironmentError, EnvironmentRequest, validate_environment};
use super::error::ChatError;
use super::request::{ChatRequest, Prepared, Style};
use super::transform::{
    async_transform_request, promote_extra_body, promoted_for_environment, transform_request,
};

const STREAM_FIELD: &str = "stream";
const JSON_MODE_FIELD: &str = "json_mode";
const IS_VERTEX_REQUEST_FIELD: &str = "is_vertex_request";
const SENSITIVE_HEADERS: [&str; 3] = ["x-api-key", "api-key", "authorization"];

pub type CompletionResult<Client, Counter> = Result<
    ChatOutcome<<Client as StreamTransport>::Failure, Counter>,
    ChatError<<Client as StreamTransport>::Failure, <Counter as ReasoningTokenCounter>::Failure>,
>;

// The Azure override owns the environment and the request body; the parent config maps prompts and replies
pub fn completion<'call, Config, Client, Counter>(
    request: ChatRequest<'call>,
    mut deps: ChatDeps<'call, Config, Client, Counter>,
) -> LocalBoxFuture<'call, CompletionResult<Client, Counter>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    async move {
        let prepared = prepare(&request, &mut deps).await?;
        if prepared.stream {
            super::stream::open(request, prepared, deps).await
        } else {
            super::buffered::send(request, prepared, deps).await
        }
    }
    .boxed_local()
}

async fn prepare<'call, Config, Client, Counter>(
    request: &ChatRequest<'call>,
    deps: &mut ChatDeps<'call, Config, Client, Counter>,
) -> Result<Prepared, ChatError<Client::Failure, Counter::Failure>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter,
{
    let mut optional_params = request.optional_params.clone();
    let stream = pop_bool(&mut optional_params, STREAM_FIELD)?.unwrap_or(false);
    let json_mode = pop_bool(&mut optional_params, JSON_MODE_FIELD)?.unwrap_or(false);
    let is_vertex_request =
        pop_bool(&mut optional_params, IS_VERTEX_REQUEST_FIELD)?.unwrap_or(false);
    let headers = request_headers(request, &optional_params, is_vertex_request)?;
    // the handler's transform promotes the caller's dict in place, and the response mapper reads it afterwards
    promote_extra_body(&mut optional_params);
    let transform = TransformRequest {
        model: request.model,
        messages: &request.messages,
        optional_params: &optional_params,
        litellm_params: &request.litellm_params,
        headers: &headers,
    };
    let (mut body, header_values) = transformed(deps.config, request.style, &transform).await?;
    // LiteLLM sets data["stream"] after filtering, and the stored pre_call body shares that dict
    if stream {
        body.insert(STREAM_FIELD.to_owned(), Json::Bool(true));
    }
    let header_map = header_map(&header_values)?;
    log_pre_call(request, deps, &body, &header_values)?;
    Ok(Prepared {
        body,
        header_map,
        optional_params,
        json_mode,
        stream,
    })
}

fn request_headers(
    request: &ChatRequest<'_>,
    optional_params: &IndexMap<String, Json>,
    is_vertex_request: bool,
) -> Result<AnthropicHeaders, EnvironmentError> {
    let probe_options = promoted_for_environment(optional_params, is_vertex_request);
    Ok(validate_environment(&EnvironmentRequest {
        caller_headers: request.caller_headers.clone(),
        params_key: request.params_key,
        argument_key: request.argument_key,
        configured_key: request.configured_key,
        token: &request.token,
        messages: &request.messages,
        optional_params: &probe_options,
    })?)
}

// The parent mapper produces the body; the filter runs on the header set and body the caller will send
async fn transformed<Config, ClientFailure, CounterFailure>(
    config: &Config,
    style: Style,
    transform: &TransformRequest<'_>,
) -> Result<
    (IndexMap<String, Json>, IndexMap<String, String>),
    ChatError<ClientFailure, CounterFailure>,
>
where
    Config: ChatConfig,
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    let mut body = match style {
        Style::Async => async_transform_request(config, transform)
            .await
            .map_err(|source| ChatError::Call(ChatCallError::Prepare { source }))?,
        Style::Sync => transform_request(config, transform)
            .map_err(|source| ChatError::Call(ChatCallError::Prepare { source }))?,
    };
    let mut header_values = transform.headers.values.clone();
    config
        .filter_request_betas(CUSTOM_LLM_PROVIDER, &mut header_values, &mut body)
        .map_err(|source| ChatError::Call(ChatCallError::Filter { source }))?;
    Ok((body, header_values))
}

fn log_pre_call<Config, Client, Counter>(
    request: &ChatRequest<'_>,
    deps: &mut ChatDeps<'_, Config, Client, Counter>,
    body: &IndexMap<String, Json>,
    headers: &IndexMap<String, String>,
) -> Result<(), ChatError<Client::Failure, Counter::Failure>>
where
    Client: StreamTransport,
    Counter: ReasoningTokenCounter,
{
    pre_call(
        &mut *deps.state,
        &PreCall {
            input: Json::List(request.messages.clone()),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(request.api_base.to_owned()),
                headers: Some(headers_args(headers)),
                complete_input_dict: Some(RawBody::Object(body.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        deps.env,
    )?;
    Ok(())
}

// LiteLLM pops each switch by name; a present null and an absent key both keep the default
fn pop_bool(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<bool>, ParameterError> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(value)),
        Some(_) => Err(ParameterError { field }),
    }
}

fn headers_args(headers: &IndexMap<String, String>) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// Header values are marked sensitive so the request's Debug output and the error keep the secret out
fn header_map<ClientFailure, CounterFailure>(
    headers: &IndexMap<String, String>,
) -> Result<HeaderMap, ChatError<ClientFailure, CounterFailure>>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    headers
        .iter()
        .map(|(name, value)| sensitive_header(name, value))
        .collect()
}

fn sensitive_header<ClientFailure, CounterFailure>(
    name: &str,
    value: &str,
) -> Result<(HeaderName, HeaderValue), ChatError<ClientFailure, CounterFailure>>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    let header_name = HeaderName::try_from(name)
        .map_err(|source| ChatError::Call(ChatCallError::HeaderName(source)))?;
    let mut header_value = HeaderValue::try_from(value)
        .map_err(|source| ChatError::Call(ChatCallError::HeaderValue(source)))?;
    header_value.set_sensitive(SENSITIVE_HEADERS.contains(&header_name.as_str()));
    Ok((header_name, header_value))
}
