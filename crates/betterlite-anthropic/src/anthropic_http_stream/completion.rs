use std::error::Error as StdError;
use std::time::Duration;

use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;

use crate::headers::response::process_anthropic_headers;
use crate::stream_decode::ChunkStream;
use crate::tool_names::REVERSE_MAP_KEY;
use crate::usage::ReasoningTokenCounter;

use super::call::{StreamBuild, StreamCall, StreamClients, StreamOpening};
use super::error::StreamOpenError;
use super::transport::{LineChunks, StreamTransport};

pub struct CompletionStream<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: IndexMap<String, Json>,
    pub messages: &'call Json,
    pub timeout: Duration,
    pub json_mode: bool,
    pub optional_params: &'call IndexMap<String, Json>,
    pub litellm_params: &'call IndexMap<String, Json>,
}

// The part of CustomStreamWrapper this slice reaches: the stream plus what acompletion_stream_function sets on it.
pub struct WrappedStream<Failure, Counter> {
    pub completion_stream: ChunkStream<LineChunks<Failure>, Counter>,
    pub model: String,
    pub custom_llm_provider: &'static str,
    pub response_headers: IndexMap<String, String>,
}

#[derive(Debug, Error)]
#[error("the {field} parameter has the wrong type")]
pub struct ParameterError {
    pub field: &'static str,
}

#[derive(Debug, Error)]
pub enum CompletionError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Parameter(#[from] ParameterError),
    #[error(transparent)]
    Open(#[from] StreamOpenError<Failure>),
}

pub async fn acompletion_stream_function<Client, Counter>(
    clients: StreamClients<'_, Client>,
    stream: CompletionStream<'_>,
    build: StreamBuild<'_, Counter>,
) -> Result<WrappedStream<Client::Failure, Counter>, CompletionError<Client::Failure>>
where
    Client: StreamTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let speed = speed_of(stream.optional_params)?;
    let reverse_map = reverse_map_of(stream.litellm_params)?;
    let mut data = stream.data;
    // data["stream"] = True replaces the key in place, so a caller's stream keeps its position in the JSON text
    data.insert("stream".to_owned(), Json::Bool(true));
    let body = python_json::dumps_object(&data);
    let call = StreamCall {
        api_base: stream.api_base,
        headers: stream.headers,
        data: &body,
        messages: stream.messages,
        timeout: stream.timeout,
        json_mode: stream.json_mode,
        speed: speed.as_deref(),
        tool_name_reverse_map: &reverse_map,
    };
    let opened = StreamOpening {
        clients,
        call: &call,
        build,
    }
    .make_call()
    .await?;
    Ok(WrappedStream {
        completion_stream: opened.completion_stream,
        model: stream.model.to_owned(),
        custom_llm_provider: "anthropic",
        response_headers: process_anthropic_headers(&opened.headers),
    })
}

fn speed_of(params: &IndexMap<String, Json>) -> Result<Option<String>, ParameterError> {
    match params.get("speed") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(speed)) => Ok(Some(speed.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ParameterError { field: "speed" })
        }
    }
}

fn reverse_map_of(
    params: &IndexMap<String, Json>,
) -> Result<IndexMap<String, String>, ParameterError> {
    let fields = match params.get(REVERSE_MAP_KEY) {
        None | Some(Json::Null) => return Ok(IndexMap::new()),
        Some(Json::Object(fields)) => fields,
        Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Err(ParameterError {
                field: REVERSE_MAP_KEY,
            });
        }
    };
    let mut reverse = IndexMap::new();
    for (sanitized, original) in fields {
        let Json::Text(text) = original else {
            return Err(ParameterError {
                field: REVERSE_MAP_KEY,
            });
        };
        reverse.insert(sanitized.clone(), text.clone());
    }
    Ok(reverse)
}
