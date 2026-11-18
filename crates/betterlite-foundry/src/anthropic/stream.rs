use betterlite_anthropic::anthropic_http_stream::call::{
    StreamBuild, StreamCall, StreamClients, StreamOpening,
};
use betterlite_anthropic::anthropic_http_stream::completion::{
    CompletionStream, ParameterError, WrappedStream, acompletion_stream_function,
};
use betterlite_anthropic::chat_call::ChatDeps;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::transport::{ChatTransport, ClientKind};
use betterlite_anthropic::headers::response::process_anthropic_headers;
use betterlite_anthropic::tool_names::REVERSE_MAP_KEY;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;

use super::CUSTOM_LLM_PROVIDER;
use super::handler::CompletionResult;
use super::request::{ChatRequest, Prepared, Style};

const SPEED_FIELD: &str = "speed";

pub(super) fn open<'call, Config, Client, Counter>(
    request: ChatRequest<'call>,
    prepared: Prepared,
    deps: ChatDeps<'call, Config, Client, Counter>,
) -> LocalBoxFuture<'call, CompletionResult<Client, Counter>>
where
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    Box::pin(async move {
        let ChatDeps {
            config: _config,
            clients,
            state: _state,
            env: _env,
            stream_logger,
            counter,
            ids,
            max_event_bytes,
            max_error_body_bytes,
        } = deps;
        // async stream keeps only an AsyncHTTPHandler; make_sync_call takes whatever client it is handed
        let supplied = match request.style {
            Style::Async => clients
                .supplied
                .as_ref()
                .filter(|item| item.kind == ClientKind::Async)
                .map(|item| item.transport),
            Style::Sync => clients.supplied.as_ref().map(|item| item.transport),
        };
        let stream_clients = StreamClients {
            supplied,
            module_level_aclient: clients.pooled,
            module_level_client: clients.pooled,
            logger: stream_logger,
        };
        let build = StreamBuild {
            ids,
            counter,
            max_event_bytes,
            max_error_body_bytes,
        };
        let messages = Json::List(request.messages);
        if request.style == Style::Async {
            let stream = CompletionStream {
                model: request.model,
                api_base: request.api_base,
                headers: prepared.header_map,
                data: prepared.body,
                messages: &messages,
                timeout: request.timeout,
                json_mode: prepared.json_mode,
                optional_params: &prepared.optional_params,
                litellm_params: &request.litellm_params,
            };
            let wrapped = acompletion_stream_function(stream_clients, stream, build).await?;
            return Ok(ChatOutcome::Stream(Box::new(wrapped)));
        }
        let encoded = python_json::dumps_object(&prepared.body);
        let (speed, reverse) = stream_names(&prepared.optional_params, &request.litellm_params)?;
        let call = StreamCall {
            api_base: request.api_base,
            headers: prepared.header_map,
            data: &encoded,
            messages: &messages,
            timeout: request.timeout,
            json_mode: prepared.json_mode,
            speed: speed.as_deref(),
            tool_name_reverse_map: &reverse,
        };
        let opened = StreamOpening {
            clients: stream_clients,
            call: &call,
            build,
        }
        .make_sync_call()
        .await?;
        Ok(ChatOutcome::Stream(Box::new(WrappedStream {
            completion_stream: opened.completion_stream,
            model: request.model.to_owned(),
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            response_headers: process_anthropic_headers(&opened.headers),
        })))
    })
}

// make_sync_call takes the speed and the reverse name map as arguments, so this branch types them before the call
fn stream_names(
    optional_params: &IndexMap<String, Json>,
    litellm_params: &IndexMap<String, Json>,
) -> Result<(Option<String>, IndexMap<String, String>), ParameterError> {
    let speed = match optional_params.get(SPEED_FIELD) {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ParameterError { field: SPEED_FIELD });
        }
    };
    Ok((speed, reverse_names(litellm_params)?))
}

fn reverse_names(
    litellm_params: &IndexMap<String, Json>,
) -> Result<IndexMap<String, String>, ParameterError> {
    let entries = match litellm_params.get(REVERSE_MAP_KEY) {
        None | Some(Json::Null) => return Ok(IndexMap::new()),
        Some(Json::Object(entries)) => entries,
        Some(_) => {
            return Err(ParameterError {
                field: REVERSE_MAP_KEY,
            });
        }
    };
    let mut names = IndexMap::with_capacity(entries.len());
    for (sanitized, original) in entries {
        let Json::Text(original) = original else {
            return Err(ParameterError {
                field: REVERSE_MAP_KEY,
            });
        };
        names.insert(sanitized.clone(), original.clone());
    }
    Ok(names)
}
