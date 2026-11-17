use std::error::Error as StdError;
use std::time::Duration;

use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::anthropic_http_stream::call::{
    StreamBuild, StreamCall, StreamClients, StreamLogger, StreamOpening,
};
use crate::anthropic_http_stream::completion::{
    CompletionError, CompletionStream, ParameterError, WrappedStream, acompletion_stream_function,
};
use crate::chat_call::error::ChatCallError;
use crate::chat_call::transport::{ChatTransport, ClientKind};
use crate::chat_call::{ChatClients, ChatOutcome};
use crate::headers::response::process_anthropic_headers;
use crate::tool_names::REVERSE_MAP_KEY;
use crate::usage::ReasoningTokenCounter;

// The sent request the two stream branches share; the body already carries stream=True and pre_call saw it
pub(crate) struct StreamFacts<'facts> {
    pub(crate) model: &'facts str,
    pub(crate) api_base: &'facts str,
    pub(crate) messages: &'facts Json,
    pub(crate) data: IndexMap<String, Json>,
    pub(crate) headers: HeaderMap,
    pub(crate) timeout: Duration,
    pub(crate) json_mode: bool,
    pub(crate) optional_params: &'facts IndexMap<String, Json>,
    pub(crate) litellm_params: &'facts IndexMap<String, Json>,
}

// acompletion_stream_function accepts only an AsyncHTTPHandler, while make_sync_call takes any supplied client
pub(crate) async fn open_stream<Client, Counter>(
    clients: ChatClients<'_, Client>,
    logger: &dyn StreamLogger,
    build: StreamBuild<'_, Counter>,
    facts: StreamFacts<'_>,
    acompletion: bool,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let supplied = clients
        .supplied
        .as_ref()
        .filter(|item| !acompletion || item.kind == ClientKind::Async)
        .map(|item| item.transport);
    let wrapping = StreamClients {
        supplied,
        module_level_aclient: clients.pooled,
        module_level_client: clients.pooled,
        logger,
    };
    if !acompletion {
        let wrapped = send_sync(wrapping, facts, build).await?;
        return Ok(ChatOutcome::Stream(Box::new(wrapped)));
    }
    let wrapped = acompletion_stream_function(wrapping, async_request(facts), build)
        .await
        .map_err(completion_error)?;
    Ok(ChatOutcome::Stream(Box::new(wrapped)))
}

// The async helper sends the request its own struct carries through the module-level async client
const fn async_request(facts: StreamFacts<'_>) -> CompletionStream<'_> {
    CompletionStream {
        model: facts.model,
        api_base: facts.api_base,
        headers: facts.headers,
        data: facts.data,
        messages: facts.messages,
        timeout: facts.timeout,
        json_mode: facts.json_mode,
        optional_params: facts.optional_params,
        litellm_params: facts.litellm_params,
    }
}

// make_sync_call sends the encoded body and wraps the opened stream with the provider name and its headers
async fn send_sync<Client, Counter>(
    clients: StreamClients<'_, Client>,
    facts: StreamFacts<'_>,
    build: StreamBuild<'_, Counter>,
) -> Result<WrappedStream<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let model = facts.model.to_owned();
    let body = python_json::dumps_object(&facts.data);
    let (speed, reverse) = stream_names(facts.optional_params, facts.litellm_params)?;
    let request = StreamCall {
        api_base: facts.api_base,
        headers: facts.headers,
        data: &body,
        messages: facts.messages,
        timeout: facts.timeout,
        json_mode: facts.json_mode,
        speed: speed.as_deref(),
        tool_name_reverse_map: &reverse,
    };
    let opened = StreamOpening {
        clients,
        call: &request,
        build,
    }
    .make_sync_call()
    .await?;
    Ok(WrappedStream {
        completion_stream: opened.completion_stream,
        model,
        custom_llm_provider: "anthropic",
        response_headers: process_anthropic_headers(&opened.headers),
    })
}

const fn completion_error<ClientFailure, CounterFailure>(
    source: CompletionError<ClientFailure>,
) -> ChatCallError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    match source {
        CompletionError::Parameter(error) => ChatCallError::Parameter(error),
        CompletionError::Open(error) => ChatCallError::Open(error),
    }
}

// make_sync_call takes the speed and the reverse name map as arguments, so this branch types them before the call
fn stream_names(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
) -> Result<(Option<String>, IndexMap<String, String>), ParameterError> {
    let speed = match optional_params.get("speed") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => return Err(ParameterError { field: "speed" }),
    };
    let reverse = match litellm_params.get(REVERSE_MAP_KEY) {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(entries)) => entries
            .iter()
            .map(|(sanitized, original)| match original {
                Json::Text(text) => Ok((sanitized.clone(), text.clone())),
                _ => Err(ParameterError {
                    field: REVERSE_MAP_KEY,
                }),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(ParameterError {
                field: REVERSE_MAP_KEY,
            });
        }
    };
    Ok((speed, reverse))
}
