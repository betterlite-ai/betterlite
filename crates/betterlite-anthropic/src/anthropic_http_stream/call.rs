use std::error::Error as StdError;
use std::time::Duration;

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

// LiteLLM hands post_call the live iterator object; the record carries the text standing for it
const STREAM_RESPONSE: &str = "lines";
const SYNC_STREAM_RESPONSE: &str = "first stream response received";

pub struct OpenedStream<Failure, Counter> {
    pub completion_stream: ChunkStream<LineChunks<Failure>, Counter>,
    pub headers: HeaderMap,
    pub sync_stream: bool,
}

pub struct StreamClients<'clients, Client> {
    pub supplied: Option<&'clients Client>,
    pub module_level_aclient: &'clients Client,
    pub module_level_client: &'clients Client,
    pub logger: &'clients dyn StreamLogger,
}

// what make_call hands logging_obj.post_call once the iterator is built
pub struct StreamPostCall<'call> {
    pub input: &'call Json,
    pub original_response: &'call str,
    pub complete_input_dict: &'call str,
}

// make_call runs the hook itself, and its future stays Send because the logger is Send + Sync
pub trait StreamLogger: Send + Sync {
    fn post_call(&self, call: StreamPostCall<'_>);
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

pub struct StreamBuild<'ids, Counter> {
    pub ids: &'ids dyn IdSource,
    pub counter: Counter,
    pub max_event_bytes: usize,
    pub max_error_body_bytes: usize,
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
    let opened = assemble(reply, call, build, false);
    clients.logger.post_call(StreamPostCall {
        input: call.messages,
        original_response: STREAM_RESPONSE,
        complete_input_dict: call.data,
    });
    Ok(opened)
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
        let message = drain_body(&mut reply.lines, build.max_error_body_bytes)
            .await
            .map_err(|source| StreamOpenError::Reraised { source })?;
        return Err(StreamOpenError::Anthropic {
            status_code: reply.status_code,
            message,
            headers: Some(reply.headers),
        });
    }
    let opened = assemble(reply, call, build, true);
    clients.logger.post_call(StreamPostCall {
        input: call.messages,
        original_response: SYNC_STREAM_RESPONSE,
        complete_input_dict: call.data,
    });
    Ok(opened)
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

async fn drain_body<Failure>(
    lines: &mut LineChunks<Failure>,
    limit: usize,
) -> Result<String, Failure>
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
        // the transport bounds one line; this bounds a reply of many short lines, stopping at a whole line
        let separator = usize::from(!text.is_empty());
        if text
            .len()
            .saturating_add(separator)
            .saturating_add(line.len())
            > limit
        {
            break;
        }
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

fn message_text(body: Vec<u8>) -> String {
    String::from_utf8(body)
        .unwrap_or_else(|error| format!("{} bytes that are not text", error.as_bytes().len()))
}
