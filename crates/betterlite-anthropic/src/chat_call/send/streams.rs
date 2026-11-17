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
pub(super) struct StreamFacts<'facts> {
    pub(super) model: &'facts str,
    pub(super) api_base: &'facts str,
    pub(super) messages: &'facts Json,
    pub(super) data: IndexMap<String, Json>,
    pub(super) headers: HeaderMap,
    pub(super) timeout: Duration,
    pub(super) json_mode: bool,
    pub(super) optional_params: &'facts IndexMap<String, Json>,
    pub(super) litellm_params: &'facts IndexMap<String, Json>,
}

pub(super) async fn open_async<Client, Counter>(
    clients: ChatClients<'_, Client>,
    logger: &dyn StreamLogger,
    build: StreamBuild<'_, Counter>,
    facts: StreamFacts<'_>,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    // acompletion_stream_function accepts only an AsyncHTTPHandler, so a supplied sync client is ignored
    let supplied = clients
        .supplied
        .as_ref()
        .filter(|item| item.kind == ClientKind::Async)
        .map(|item| item.transport);
    let stream = CompletionStream {
        model: facts.model,
        api_base: facts.api_base,
        headers: facts.headers,
        data: facts.data,
        messages: facts.messages,
        timeout: facts.timeout,
        json_mode: facts.json_mode,
        optional_params: facts.optional_params,
        litellm_params: facts.litellm_params,
    };
    let wrapping = StreamClients {
        supplied,
        module_level_aclient: clients.pooled,
        module_level_client: clients.pooled,
        logger,
    };
    let wrapped = acompletion_stream_function(wrapping, stream, build)
        .await
        .map_err(completion_error)?;
    Ok(ChatOutcome::Stream(Box::new(wrapped)))
}

pub(super) async fn open_sync<Client, Counter>(
    clients: ChatClients<'_, Client>,
    logger: &dyn StreamLogger,
    build: StreamBuild<'_, Counter>,
    facts: StreamFacts<'_>,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let supplied = clients.supplied.as_ref().map(|item| item.transport);
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
        clients: StreamClients {
            supplied,
            module_level_aclient: clients.pooled,
            module_level_client: clients.pooled,
            logger,
        },
        call: &request,
        build,
    }
    .make_sync_call()
    .await?;
    Ok(ChatOutcome::Stream(Box::new(WrappedStream {
        completion_stream: opened.completion_stream,
        model: facts.model.to_owned(),
        custom_llm_provider: "anthropic",
        response_headers: process_anthropic_headers(&opened.headers),
    })))
}

fn completion_error<ClientFailure, CounterFailure>(
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
