use std::error::Error as StdError;
use std::future::Future;

use betterlite_anthropic::anthropic_http_stream::completion::ParameterError;
use betterlite_anthropic::anthropic_http_stream::transport::StreamTransport;
use betterlite_anthropic::chat_call::CallFacts;
use betterlite_anthropic::chat_call::CallStyle;
use betterlite_anthropic::chat_call::ChatDeps;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::SyncStream;
use betterlite_anthropic::chat_call::SyncStreamNames;
use betterlite_anthropic::chat_call::config::{ChatConfig, TransformOutcome, TransformRequest};
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::prepare::Prepared;
use betterlite_anthropic::chat_call::steps::finish;
use betterlite_anthropic::chat_call::transport::ChatTransport;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::BillingHeader;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::environment::{EnvironmentError, EnvironmentRequest, validate_environment};
use super::error::ChatError;
use super::request::{ChatRequest, Style};
use super::transform::{promote_extra_body, promoted_for_environment, transform_request};
use super::{CUSTOM_LLM_PROVIDER, SHOULD_STRIP_BILLING_METADATA};

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
) -> impl Future<Output = CompletionResult<Client, Counter>> + Send + 'call
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    // The request's credential sources and token provider are not Send, so prepare runs before the future exists
    let prepared = prepare(&request, &mut deps);
    let facts = CallFacts {
        model: request.model,
        messages: request.messages,
        litellm_params: request.litellm_params,
        api_base: request.api_base,
        timeout: request.timeout,
        style: match request.style {
            Style::Async => CallStyle::Async,
            Style::Sync => CallStyle::Sync,
        },
        now: request.now,
        disable_add_prefix_to_prompt: request.disable_add_prefix_to_prompt,
        model_response: request.model_response,
        // the handler makes its sync call without the caller's speed or reverse map and stamps azure_ai
        sync_stream: SyncStream {
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            names: SyncStreamNames::Omitted,
        },
    };
    async move {
        finish(facts, prepared?, deps)
            .await
            .map_err(ChatError::Call)
    }
}

fn prepare<'call, Config, Client, Counter>(
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
        custom_llm_provider: Some(CUSTOM_LLM_PROVIDER),
        messages: &request.messages,
        optional_params: &optional_params,
        litellm_params: &request.litellm_params,
        headers: &headers,
        billing_header: BillingHeader::from(SHOULD_STRIP_BILLING_METADATA),
    };
    let TransformOutcome {
        mut body,
        headers: mapped,
    } = transformed(deps.config, &transform)?;
    // LiteLLM sets data["stream"] after filtering, and the stored pre_call body shares that dict
    if stream {
        body.insert(STREAM_FIELD.to_owned(), Json::Bool(true));
    }
    let header_values = mapped.values;
    let header_map = header_map(&header_values)?;
    log_pre_call(request, deps, &body, &header_values)?;
    Ok(Prepared {
        body,
        header_map,
        optional_params,
        stream,
        json_mode,
    })
}

fn request_headers(
    request: &ChatRequest<'_>,
    optional_params: &IndexMap<String, Json>,
    is_vertex_request: bool,
) -> Result<AnthropicHeaders, EnvironmentError> {
    let probe_options = promoted_for_environment(optional_params, is_vertex_request);
    validate_environment(&EnvironmentRequest {
        caller_headers: request.caller_headers.clone(),
        params_key: request.params_key,
        argument_key: request.argument_key,
        configured_key: request.configured_key,
        token: &request.token,
        messages: &request.messages,
        optional_params: &probe_options,
    })
}

// Azure uses the parent mapper, then sends its updated headers without the Anthropic beta filter
fn transformed<Config, ClientFailure, CounterFailure>(
    config: &Config,
    transform: &TransformRequest<'_>,
) -> Result<TransformOutcome, ChatError<ClientFailure, CounterFailure>>
where
    Config: ChatConfig,
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    transform_request(config, transform)
        .map_err(|source| ChatError::Call(ChatCallError::Prepare { source }))
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
