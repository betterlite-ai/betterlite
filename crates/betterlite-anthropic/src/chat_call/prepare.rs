use std::error::Error as StdError;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use futures_util::future::LocalBoxFuture;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::anthropic_http_stream::completion::ParameterError;
use crate::anthropic_http_stream::transport::StreamTransport;
use crate::headers::AnthropicHeaders;
use crate::headers::environment::{
    EnvironmentRequest, HeaderLiteLlmParams, HeaderParams, validate_environment,
};
use crate::request::BillingHeader;
use crate::usage::ReasoningTokenCounter;

use super::config::{ChatConfig, TransformOutcome, TransformRequest};
use super::error::{ChatCallError, ConfigError};
use super::transport::ChatTransport;
use super::{CallStyle, ChatCall, ChatDeps};

pub struct Prepared {
    pub body: IndexMap<String, Json>,
    pub header_map: HeaderMap,
    pub optional_params: IndexMap<String, Json>,
    pub stream: bool,
    pub json_mode: bool,
}
struct Params {
    optional: IndexMap<String, Json>,
    transform: IndexMap<String, Json>,
    is_vertex_request: bool,
}
type PrepareResult<Client, Counter> = Result<
    Prepared,
    ChatCallError<
        <Client as StreamTransport>::Failure,
        <Counter as ReasoningTokenCounter>::Failure,
    >,
>;

pub(super) fn prepare<'request, 'deps, Config, Client, Counter>(
    call: &'request ChatCall<'_>,
    deps: &'request mut ChatDeps<'deps, Config, Client, Counter>,
) -> LocalBoxFuture<'request, PrepareResult<Client, Counter>>
where
    'deps: 'request,
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter,
{
    Box::pin(async move {
        let mut optional_params = call.optional_params.clone();
        let stream = take_bool(&mut optional_params, "stream", false)?;
        let json_mode = take_bool(&mut optional_params, "json_mode", false)?;
        let params = take_params(optional_params)?;
        let validated = validated_headers(call, &params.optional, params.is_vertex_request)?;
        let transform = TransformRequest {
            model: call.model,
            custom_llm_provider: Some(call.custom_llm_provider),
            messages: &call.messages,
            optional_params: &params.transform,
            litellm_params: &call.litellm_params,
            headers: &validated,
            billing_header: BillingHeader::Keep,
        };
        let TransformOutcome {
            mut body,
            headers: mapped,
        } = map_request(deps.config, &transform, call.style)
            .await
            .map_err(|source| ChatCallError::Prepare { source })?;
        let mut headers = mapped.values;
        deps.config
            .filter_request_betas(call.custom_llm_provider, &mut headers, &mut body)
            .map_err(|source| ChatCallError::Filter { source })?;
        // LiteLLM sets data["stream"] after filtering, and its stored pre_call body shares that dict and shows the key
        if stream {
            body.insert("stream".to_owned(), Json::Bool(true));
        }
        let header_map = header_map(&headers)?;
        log_pre_call(call, &mut *deps.state, deps.env, &body, &headers)?;
        Ok(Prepared {
            body,
            header_map,
            optional_params: params.optional,
            stream,
            json_mode,
        })
    })
}
async fn map_request<Config>(
    config: &Config,
    request: &TransformRequest<'_>,
    style: CallStyle,
) -> Result<TransformOutcome, ConfigError>
where
    Config: ChatConfig,
{
    if style == CallStyle::Async {
        config.async_transform_outcome(request).await
    } else {
        config.transform_outcome(request)
    }
}
fn take_params(mut popped: IndexMap<String, Json>) -> Result<Params, ParameterError> {
    let is_vertex_request = take_bool(&mut popped, "is_vertex_request", false)?;
    let _vertex_location = popped.shift_remove("vertex_count_tokens_location");
    let mut transform_params = popped.clone();
    transform_params.insert(
        "is_vertex_request".to_owned(),
        Json::Bool(is_vertex_request),
    );
    Ok(Params {
        optional: popped,
        transform: transform_params,
        is_vertex_request,
    })
}
fn validated_headers<ClientFailure, CounterFailure>(
    call: &ChatCall<'_>,
    optional_params: &IndexMap<String, Json>,
    is_vertex_request: bool,
) -> Result<AnthropicHeaders, ChatCallError<ClientFailure, CounterFailure>>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    let header_options = HeaderParams {
        tools: option_list(optional_params, "tools")?,
        mcp_servers: option_list(optional_params, "mcp_servers")?,
        thinking: non_null(optional_params, "thinking").map(prompt_json),
        is_vertex_request,
        container: non_null(optional_params, "container").cloned(),
    };
    let messages: Vec<PromptJson> = call.messages.iter().map(prompt_json).collect();
    let litellm_params = lite_llm_params(&call.litellm_params)?;
    validate_environment(
        &EnvironmentRequest {
            model: call.model,
            caller_headers: &call.caller_headers,
            api_key: call.api_key,
            api_base: Some(call.api_base),
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &header_options,
            effort_used: call.effort_used,
        },
        call.settings,
    )
    .map_err(ChatCallError::from)
}
fn log_pre_call<ClientFailure, CounterFailure>(
    call: &ChatCall<'_>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    body: &IndexMap<String, Json>,
    headers: &IndexMap<String, String>,
) -> Result<(), ChatCallError<ClientFailure, CounterFailure>>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    pre_call(
        state,
        &PreCall {
            input: Json::List(call.messages.clone()),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(call.api_base.to_owned()),
                headers: Some(headers_args(headers)),
                complete_input_dict: Some(RawBody::Object(body.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        env,
    )?;
    Ok(())
}
fn take_bool(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
    default: bool,
) -> Result<bool, ParameterError> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(default),
        Some(Json::Bool(value)) => Ok(value),
        Some(_) => Err(ParameterError { field }),
    }
}
fn option_list(
    options: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<Vec<PromptJson>>, ParameterError> {
    match non_null(options, field) {
        None => Ok(None),
        Some(Json::List(items)) => Ok(Some(items.iter().map(prompt_json).collect())),
        Some(_) => Err(ParameterError { field }),
    }
}
fn non_null<'options>(
    options: &'options IndexMap<String, Json>,
    field: &str,
) -> Option<&'options Json> {
    options
        .get(field)
        .filter(|value| !matches!(value, Json::Null))
}
fn lite_llm_params(params: &IndexMap<String, Json>) -> Result<HeaderLiteLlmParams, ParameterError> {
    let api_base = match params.get("api_base") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ParameterError { field: "api_base" });
        }
    };
    let use_bearer_for_custom_base = match params.get("use_bearer_for_custom_base") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(value)) => *value,
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ParameterError {
                field: "use_bearer_for_custom_base",
            });
        }
    };
    Ok(HeaderLiteLlmParams {
        api_base,
        use_bearer_for_custom_base,
    })
}
// LiteLLM passes the same values to the header checks and to the request transform, which keeps them as sent
fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), prompt_json(item)))
                .collect(),
        ),
    }
}
fn headers_args(headers: &IndexMap<String, String>) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
fn header_map<ClientFailure, CounterFailure>(
    headers: &IndexMap<String, String>,
) -> Result<HeaderMap, ChatCallError<ClientFailure, CounterFailure>>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            HeaderName::try_from(name.as_str())?,
            HeaderValue::try_from(value.as_str())?,
        );
    }
    Ok(map)
}
