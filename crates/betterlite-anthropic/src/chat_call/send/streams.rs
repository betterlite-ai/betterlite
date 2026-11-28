use std::time::Duration;

use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::anthropic_http_stream::call::{
    StreamBuild, StreamCall, StreamClients, StreamLogger, StreamOpening,
};
use crate::anthropic_http_stream::completion::{
    CompletionStream, ParameterError, WrappedStream, acompletion_stream_function,
};
use crate::chat_call::error::ChatCallError;
use crate::chat_call::transport::ChatTransport;
use crate::chat_call::{ChatClients, ChatOutcome, SyncStreamNames};
use crate::headers::response::process_anthropic_headers;
use crate::tool_names::REVERSE_MAP_KEY;
use crate::usage::ReasoningTokenCounter;
use betterlite_http::client_kind::ClientKind;

pub struct StreamFacts<'facts> {
    pub model: &'facts str,
    pub api_base: &'facts str,
    pub messages: &'facts Json,
    pub data: IndexMap<String, Json>,
    pub headers: HeaderMap,
    pub timeout: Duration,
    pub json_mode: bool,
    pub optional_params: &'facts IndexMap<String, Json>,
    pub litellm_params: &'facts IndexMap<String, Json>,
    // The sync wrapper stamps the provider its handler passes; the async helper always stamps anthropic
    pub custom_llm_provider: &'static str,
    // Whether make_sync_call receives the caller's speed and reverse map; Azure's override passes neither
    pub names: SyncStreamNames,
}
pub async fn open_stream<Client, Counter>(
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
        .filter(|item| !acompletion || item.kind == ClientKind::Asynchronous)
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
    let wrapped =
        acompletion_stream_function(wrapping, CompletionStream::from(facts), build).await?;
    Ok(ChatOutcome::Stream(Box::new(wrapped)))
}
// The async helper sends the request its own struct carries through the module-level async client
impl<'facts> From<StreamFacts<'facts>> for CompletionStream<'facts> {
    fn from(facts: StreamFacts<'facts>) -> Self {
        Self {
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
    // make_sync_call takes the speed and the reverse name map as arguments, so this branch types them first
    let (speed, reverse) = match facts.names {
        SyncStreamNames::FromParams => stream_names(facts.optional_params, facts.litellm_params)?,
        SyncStreamNames::Omitted => (None, IndexMap::new()),
    };
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
        custom_llm_provider: facts.custom_llm_provider,
        response_headers: process_anthropic_headers(&opened.headers),
    })
}
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
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(ParameterError {
                        field: REVERSE_MAP_KEY,
                    })
                }
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
