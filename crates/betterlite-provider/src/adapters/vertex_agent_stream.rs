pub mod transport;

mod tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use betterlite_bedrock::converse_events::wire::non_empty;
use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::aleph_alpha::lines::{LineDecoder, decode_text};
use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::chat::response::ChatUsage;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};
use betterlite_prompts::message_content::Message;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use betterlite_vertex::vertex_agent_engine::error::{AgentEngineError, get_error_class};
use betterlite_vertex::vertex_agent_engine::records::AgentQueryPayload;
use betterlite_vertex::vertex_agent_stream::{
    AgentEngineChunk, AgentEngineStreamError, AgentEngineUsage, parse_chunk,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, Method};
use thiserror::Error;
use tokio::time::timeout;
use url::Url;
use uuid::Uuid;

use self::transport::{Chunks, StreamHead, StreamTransport};

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const DONE_MARKER: &str = "[DONE]";
pub const FINISH_STOP: &str = "stop";
pub const FIRST_STREAM_RESPONSE: &str = "first stream response received";
pub const MAX_BODY_BYTES: usize = 0x0100_0000;
pub const MAX_LINE_BYTES: usize = TransportLimits::DEFAULT.body_bytes;
pub const OK_STATUS: u16 = 200;
pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(600);

const ERROR_LOG: &str = "Error in Vertex Agent Engine SSE stream";

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
    ids: Arc<dyn IdSource>,
    // the body stream owns the HTTP response, so dropping it here releases the connection
    chunks: Chunks,
    decoder: LineDecoder,
    pending: VecDeque<String>,
    done: bool,
}

impl VertexAgentStream {
    pub async fn next_chunk(&mut self) -> Option<Chunk> {
        loop {
            if let Some(line) = self.pending.pop_front() {
                match parse_line(&line, &self.model, self.ids.as_ref()) {
                    Ok(Some(chunk)) => return Some(chunk),
                    Ok(None) => {}
                    Err(error) => return self.fail(&error),
                }
                continue;
            }
            if self.done {
                return None;
            }
            match next_frame(&mut self.chunks).await {
                Ok(Some(bytes)) => {
                    if let Err(source) = self.decoder.push(&bytes, &mut self.pending) {
                        return self.fail(&VertexAgentStreamError::Line(source));
                    }
                }
                Ok(None) => {
                    self.decoder.flush(&mut self.pending);
                    self.done = true;
                }
                Err(error) => return self.fail(&error),
            }
        }
    }

    // LiteLLM reports a broken stream by raising out of its iterator; here the read ends instead
    fn fail(&mut self, error: &VertexAgentStreamError) -> Option<Chunk> {
        tracing::error!("{ERROR_LOG}: {error}");
        self.chunks = stream::empty().boxed();
        self.pending.clear();
        self.done = true;
        None
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
    match timeout(STREAMING_TIMEOUT, chunks.next()).await {
        Ok(Some(Ok(bytes))) => Ok(Some(bytes)),
        Ok(Some(Err(source))) => Err(VertexAgentStreamError::Body { source }),
        Ok(None) => Ok(None),
        Err(_elapsed) => Err(VertexAgentStreamError::Timeout),
    }
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

fn parse_line(
    line: &str,
    model: &str,
    ids: &dyn IdSource,
) -> Result<Option<Chunk>, VertexAgentStreamError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    if line.contains(DONE_MARKER) {
        return Ok(Some(done_chunk(model, ids)));
    }
    let payload = strip_sse_data_from_chunk(Some(line)).unwrap_or(line);
    let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
        tracing::debug!("Skipping a Vertex Agent Engine stream line that is not JSON");
        return Ok(None);
    };
    if !non_empty(&parsed) {
        return Ok(None);
    }
    let Json::Object(fields) = parsed else {
        return Err(VertexAgentStreamError::ChunkNotObject);
    };
    Ok(Some(chunk_of(model, ids, &parse_chunk(&fields)?)))
}

// the generic iterator answers the done marker with a finished chunk and no text
fn done_chunk(model: &str, ids: &dyn IdSource) -> Chunk {
    chunk_of(
        model,
        ids,
        &AgentEngineChunk {
            content: None,
            role: None,
            finish_reason: Some(FINISH_STOP),
            usage: None,
        },
    )
}

fn chunk_of(model: &str, ids: &dyn IdSource, chunk: &AgentEngineChunk<'_>) -> Chunk {
    Chunk {
        id: format!("{CHAT_COMPLETION_ID_PREFIX}{}", ids.next_id()),
        object: CHUNK_OBJECT.to_owned(),
        created: 0,
        model: model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: chunk.role.map(str::to_owned),
                content: chunk.content.map(str::to_owned),
                refusal: None,
                tool_calls: None,
                function_call: None,
            },
            finish_reason: chunk.finish_reason.map(str::to_owned),
            logprobs: None,
        }],
        usage: chunk.usage.map(chat_usage),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn chat_usage(usage: AgentEngineUsage) -> ChatUsage {
    ChatUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}
