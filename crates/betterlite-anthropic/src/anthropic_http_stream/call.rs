use std::error::Error as StdError;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use http::HeaderMap;
use indexmap::IndexMap;
use url::Url;

use crate::stream_decode::error::ReceiveError;
use crate::stream_decode::{ChunkStream, TransportChunk};
use crate::stream_events::StreamEventConverter;
use crate::stream_state::state::{IdSource, StreamOptions, StreamState};
use crate::usage::ReasoningTokenCounter;

use super::error::{PostFailure, StreamOpenError};
use super::transport::{LineChunks, StreamPost, StreamReply, StreamTransport};

pub struct OpenedStream<Failure, Counter> {
    pub completion_stream: ChunkStream<LineChunks<Failure>, Counter>,
    pub headers: HeaderMap,
    pub sync_stream: bool,
}

pub struct StreamClients<'clients, Client> {
    pub supplied: Option<&'clients Client>,
    pub module_level_aclient: &'clients Client,
    pub module_level_client: &'clients Client,
}

pub struct StreamCall<'call> {
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: &'call str,
    pub messages: &'call Json,
    pub timeout: Duration,
    pub json_mode: bool,
    pub speed: Option<&'call str>,
    pub tool_name_reverse_map: &'call IndexMap<String, String>,
}

pub struct StreamLog<'log, 'env> {
    pub state: &'log mut LoggingState,
    pub env: &'log PrePostEnv<'env>,
}

pub struct StreamBuild<'ids, Counter> {
    pub ids: &'ids dyn IdSource,
    pub counter: Counter,
    pub max_event_bytes: usize,
}

pub async fn make_call<Client, Counter>(
    clients: StreamClients<'_, Client>,
    call: &StreamCall<'_>,
    build: StreamBuild<'_, Counter>,
) -> Result<OpenedStream<Client::Failure, Counter>, StreamOpenError<Client::Failure>>
where
    Client: StreamTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let client = clients.supplied.unwrap_or(clients.module_level_aclient);
    let reply = open(client, call).await?;
    Ok(assemble(reply, call, build, false))
}

pub async fn make_sync_call<Client, Counter>(
    clients: StreamClients<'_, Client>,
    call: &StreamCall<'_>,
    build: StreamBuild<'_, Counter>,
) -> Result<OpenedStream<Client::Failure, Counter>, StreamOpenError<Client::Failure>>
where
    Client: StreamTransport,
    Counter: ReasoningTokenCounter + Unpin + Send,
{
    let client = clients.supplied.unwrap_or(clients.module_level_client);
    let mut reply = open(client, call).await?;
    if reply.status_code != 200 {
        let message = drain_body(&mut reply.lines)
            .await
            .map_err(|source| StreamOpenError::Reraised { source })?;
        return Err(StreamOpenError::Anthropic {
            status_code: reply.status_code,
            message,
            headers: Some(reply.headers),
        });
    }
    Ok(assemble(reply, call, build, true))
}

async fn open<Client>(
    client: &Client,
    call: &StreamCall<'_>,
) -> Result<StreamReply<Client::Failure>, StreamOpenError<Client::Failure>>
where
    Client: StreamTransport,
{
    let url = Url::parse(call.api_base).map_err(|source| StreamOpenError::Endpoint { source })?;
    let reply = client
        .post(StreamPost {
            url,
            headers: call.headers.clone(),
            body: call.data.as_bytes().to_vec(),
            stream: true,
            deadline: call.timeout,
        })
        .await;
    reply.map_err(|failure| match failure {
        PostFailure::Status(status) => StreamOpenError::Anthropic {
            status_code: status.status_code,
            message: message_text(status.body),
            headers: status.exception_headers.or(status.response_headers),
        },
        PostFailure::Known(failure) => StreamOpenError::Reraised { source: failure },
        PostFailure::Unknown(failure) => StreamOpenError::Unknown { source: failure },
    })
}

async fn drain_body<Failure>(lines: &mut LineChunks<Failure>) -> Result<String, Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    let mut text = String::new();
    while let Some(item) = lines.next().await {
        let line = match item {
            Ok(TransportChunk::Text(line)) => line,
            Ok(TransportChunk::Bytes(bytes)) => message_text(bytes),
            Err(ReceiveError::Value { source } | ReceiveError::Other { source }) => {
                return Err(source);
            }
        };
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
    }
    Ok(text)
}

fn assemble<Failure, Counter>(
    reply: StreamReply<Failure>,
    call: &StreamCall<'_>,
    build: StreamBuild<'_, Counter>,
    sync_stream: bool,
) -> OpenedStream<Failure, Counter>
where
    Counter: ReasoningTokenCounter,
{
    let options = StreamOptions {
        sync_stream,
        json_mode: Some(call.json_mode),
        speed: call.speed.map(str::to_owned),
        tool_name_reverse_map: call.tool_name_reverse_map.clone(),
    };
    let converter = StreamEventConverter {
        state: StreamState::new(options, build.ids),
        counter: build.counter,
        json_could_close: false,
    };
    OpenedStream {
        completion_stream: ChunkStream {
            converter,
            source: reply.lines,
            source_ended: false,
            accumulated_bytes: 0,
            max_event_bytes: build.max_event_bytes,
        },
        headers: reply.headers,
        sync_stream,
    }
}

pub fn record_post_call(log: StreamLog<'_, '_>, messages: &Json, data: &str, sync: bool) {
    let response = if sync {
        "first stream response received"
    } else {
        "lines"
    };
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Text(response.to_owned()),
            input: messages.clone(),
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                // litellm passes the outgoing JSON text and no headers, so the credential never reaches the callback
                complete_input_dict: Some(RawBody::Text(data.to_owned())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
}

fn message_text(body: Vec<u8>) -> String {
    String::from_utf8(body)
        .unwrap_or_else(|error| format!("{} bytes that are not text", error.as_bytes().len()))
}
