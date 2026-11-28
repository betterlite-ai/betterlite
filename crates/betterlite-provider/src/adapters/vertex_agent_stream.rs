pub mod transport;

mod chunks;
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::aleph_alpha::lines::{LineDecoder, decode_text};
use betterlite_openai::stream::chunk::Chunk;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::vertex_agent_engine::error::{AgentEngineError, get_error_class};
use betterlite_vertex::vertex_agent_engine::records::AgentQueryPayload;
use betterlite_vertex::vertex_agent_stream::AgentEngineStreamError;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, Method};
use thiserror::Error;
use tokio::time::timeout;
use url::Url;
use uuid::Uuid;

use self::chunks::parse_line;
use self::transport::{Chunks, StreamHead, StreamTransport};

pub const FIRST_STREAM_RESPONSE: &str = "first stream response received";
pub const MAX_BODY_BYTES: usize = 0x0100_0000;
pub const MAX_LINE_BYTES: usize = TransportLimits::DEFAULT.body_bytes;
pub const OK_STATUS: u16 = 200;
pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(600);

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

pub struct UuidV4;

impl IdSource for UuidV4 {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

#[derive(Debug, Error)]
pub enum VertexAgentStreamError {
    #[error(transparent)]
    Chunk(#[from] AgentEngineStreamError),
    #[error("a Vertex Agent Engine stream line is not a JSON object")]
    ChunkNotObject,
    #[error("the Vertex Agent Engine stream body could not be read")]
    Body {
        #[source]
        source: HttpError,
    },
    #[error(transparent)]
    Line(#[from] AlephAlphaError),
    #[error("the Vertex Agent Engine stream timed out")]
    Timeout,
    #[error("the Vertex Agent Engine stream error body is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the Vertex Agent Engine stream could not be reached")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the Vertex Agent Engine stream request body could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex Agent Engine stream URL is not a URL")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error(transparent)]
    Provider(#[from] AgentEngineError),
}

pub struct StreamRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub url: &'request str,
    pub headers: HeaderMap,
    pub data: &'request AgentQueryPayload,
    pub messages: &'request [Message],
    pub kind: ClientKind,
}

pub struct StreamDeps<'deps> {
    pub supplied: Option<Arc<dyn StreamTransport>>,
    pub default_client: Arc<dyn StreamTransport>,
    pub ids: Arc<dyn IdSource>,
    pub logger: &'deps dyn StreamLogger,
}

pub struct StreamPostCall<'call> {
    pub input: &'call [Message],
    pub api_key: &'call str,
    pub original_response: &'static str,
    pub complete_input_dict: &'call AgentQueryPayload,
}

pub trait StreamLogger: Send + Sync {
    fn post_call(&self, call: StreamPostCall<'_>);
}

pub struct VertexAgentStream {
    pub model: String,
    pub provider: String,
    pub ids: Arc<dyn IdSource>,
    // the body stream owns the HTTP response, so dropping it here releases the connection
    pub chunks: Chunks,
    pub decoder: LineDecoder,
    pub pending: VecDeque<String>,
    pub done: bool,
    // a finished chunk ends the stream for LiteLLM's wrapper, so a later read failure (not a timeout) is not seen
    pub finished: bool,
}

impl VertexAgentStream {
    pub async fn next_chunk(&mut self) -> Result<Option<Chunk>, VertexAgentStreamError> {
        loop {
            if self.pending.is_empty() && self.done {
                return Ok(None);
            }
            let Some(line) = self.pending.pop_front() else {
                self.read_frame().await?;
                continue;
            };
            match parse_line(&line, &self.model, self.ids.as_ref()) {
                Ok(Some(chunk)) => {
                    self.finished |= chunk
                        .choices
                        .first()
                        .is_some_and(|choice| choice.finish_reason.is_some());
                    return Ok(Some(chunk));
                }
                Ok(None) => {}
                Err(error) => {
                    self.end();
                    return Err(error);
                }
            }
        }
    }

    // a read error after a finish reason ends LiteLLM's wrapper cleanly, but a timeout still raises
    async fn read_frame(&mut self) -> Result<(), VertexAgentStreamError> {
        let frame = match next_frame(&mut self.chunks).await {
            Ok(frame) => frame,
            Err(error) => {
                self.end();
                if self.finished && !timed_out(&error) {
                    return Ok(());
                }
                return Err(error);
            }
        };
        let Some(bytes) = frame else {
            self.decoder.flush(&mut self.pending);
            self.done = true;
            return Ok(());
        };
        if let Err(source) = self.decoder.push(&bytes, &mut self.pending) {
            self.end();
            return Err(VertexAgentStreamError::Line(source));
        }
        Ok(())
    }

    // LiteLLM raises a broken stream out of its iterator, so the error reaches the caller once
    fn end(&mut self) {
        self.chunks = stream::empty().boxed();
        self.pending.clear();
        self.done = true;
    }
}

// LiteLLM's wrapper ends cleanly only on a read or protocol error, a timeout still raises
fn timed_out(error: &VertexAgentStreamError) -> bool {
    match error {
        VertexAgentStreamError::Body {
            source: HttpError::Request(source),
        } => source.is_timeout(),
        VertexAgentStreamError::Body {
            source: HttpError::Timeout,
        }
        | VertexAgentStreamError::Timeout => true,
        VertexAgentStreamError::Body { .. }
        | VertexAgentStreamError::Chunk(_)
        | VertexAgentStreamError::ChunkNotObject
        | VertexAgentStreamError::Line(_)
        | VertexAgentStreamError::TooLarge { .. }
        | VertexAgentStreamError::Transport { .. }
        | VertexAgentStreamError::Encode { .. }
        | VertexAgentStreamError::Url { .. }
        | VertexAgentStreamError::Provider(_) => false,
    }
}

pub async fn open(
    request: &StreamRequest<'_>,
    deps: &StreamDeps<'_>,
) -> Result<VertexAgentStream, VertexAgentStreamError> {
    let head = client_for(deps, request.kind)
        .post(outbound(request)?)
        .await
        .map_err(|source| VertexAgentStreamError::Transport { source })?;
    let stream = VertexAgentStream {
        model: request.model.to_owned(),
        provider: request.provider.to_owned(),
        ids: Arc::clone(&deps.ids),
        chunks: accepted(head).await?,
        decoder: LineDecoder::new(MAX_LINE_BYTES),
        pending: VecDeque::new(),
        done: false,
        finished: false,
    };
    deps.logger.post_call(StreamPostCall {
        input: request.messages,
        api_key: "",
        original_response: FIRST_STREAM_RESPONSE,
        complete_input_dict: request.data,
    });
    Ok(stream)
}

// the supplied client answers only when it reads the way this call reads
fn client_for(deps: &StreamDeps<'_>, kind: ClientKind) -> Arc<dyn StreamTransport> {
    match &deps.supplied {
        Some(supplied) if supplied.kind() == kind => Arc::clone(supplied),
        Some(_) | None => Arc::clone(&deps.default_client),
    }
}

fn outbound(request: &StreamRequest<'_>) -> Result<OutboundRequest, VertexAgentStreamError> {
    let url = Url::parse(request.url).map_err(|source| VertexAgentStreamError::Url { source })?;
    let body = serde_json::to_vec(request.data)
        .map_err(|source| VertexAgentStreamError::Encode { source })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: request.headers.clone(),
        body: RequestBody::Json(body),
        deadline: TransportLimits::DEFAULT.read_timeout,
    })
}

// LiteLLM accepts exactly 200 and builds its error from the status and the whole failing body
async fn accepted(head: StreamHead) -> Result<Chunks, VertexAgentStreamError> {
    if head.status == OK_STATUS {
        return Ok(head.chunks);
    }
    let message = error_message(head.chunks).await?;
    Err(VertexAgentStreamError::Provider(get_error_class(
        &message,
        head.status,
    )))
}

// LiteLLM's iterator waits on the transport with no deadline of its own; betterlite ends a silent read
async fn next_frame(chunks: &mut Chunks) -> Result<Option<Vec<u8>>, VertexAgentStreamError> {
    let read = timeout(STREAMING_TIMEOUT, chunks.next())
        .await
        .map_err(|_elapsed| VertexAgentStreamError::Timeout)?;
    read.transpose()
        .map_err(|source| VertexAgentStreamError::Body { source })
}

// LiteLLM keeps the whole failing body as the error message text
async fn error_message(mut chunks: Chunks) -> Result<String, VertexAgentStreamError> {
    let mut body: Vec<u8> = Vec::new();
    let mut remaining = MAX_BODY_BYTES;
    loop {
        let Some(bytes) = next_frame(&mut chunks).await? else {
            return Ok(decode_text(&body));
        };
        remaining = remaining
            .checked_sub(bytes.len())
            .ok_or(VertexAgentStreamError::TooLarge {
                limit: MAX_BODY_BYTES,
            })?;
        body.extend_from_slice(&bytes);
    }
}
