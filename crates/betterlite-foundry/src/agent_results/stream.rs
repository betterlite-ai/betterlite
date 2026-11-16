use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_prompts::arguments::json::Object;
use futures_util::stream::{BoxStream, empty, unfold};
use futures_util::{Stream, StreamExt as _};
use serde::Serialize;
use uuid::Uuid;

use super::error::AgentResultError;

mod decode;

pub const CHUNK_ID_PREFIX: &str = "chatcmpl-";
pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const DATA_PREFIX: &str = "data:";
pub const DONE_DATA: &str = "[DONE]";
pub const EVENT_PREFIX: &str = "event:";
pub const MAX_ERROR_BODY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;
pub const MAX_LINE_BYTES: usize = 0x0010_0000;
pub const MESSAGE_COMPLETED_EVENT: &str = "thread.message.completed";
pub const MESSAGE_DELTA_EVENT: &str = "thread.message.delta";
pub const STATUS_CREATED: u16 = 201;
pub const STATUS_OK: u16 = 200;
pub const THREAD_CREATED_EVENT: &str = "thread.created";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamIdentity {
    pub id: String,
    pub created: i64,
}

// LiteLLM's f"chatcmpl-{uuid.uuid4().hex[:8]}" over the caller's UUID and clock
#[must_use]
pub fn response_identity(uuid: Uuid, created: i64) -> StreamIdentity {
    let simple = uuid.simple().to_string();
    let mut short = String::with_capacity(8);
    short.extend(simple.chars().take(8));
    StreamIdentity {
        id: format!("{CHUNK_ID_PREFIX}{short}"),
        created,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChunkDelta {
    pub content: Option<String>,
    pub role: Option<&'static str>,
    pub annotations: Option<Vec<Object>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub finish_reason: Option<&'static str>,
    pub delta: ChunkDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentChunk {
    pub id: String,
    pub created: i64,
    pub model: String,
    pub object: &'static str,
    pub choices: Vec<ChunkChoice>,
    pub thread_id: Option<String>,
}

pub struct StreamSetup<'request> {
    pub model: &'request str,
    pub status: u16,
    pub identity: StreamIdentity,
}

pub struct AgentChunkStream {
    inner: BoxStream<'static, Result<AgentChunk, AgentResultError>>,
}

impl AgentChunkStream {
    fn new(
        body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
        model: &str,
        identity: StreamIdentity,
    ) -> Self {
        let state = decode::ChunkState::new(body, model, identity);
        Self {
            inner: unfold(state, decode::ChunkState::advance).boxed(),
        }
    }
}

impl Stream for AgentChunkStream {
    type Item = Result<AgentChunk, AgentResultError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

// a 200 or 201 body is decoded as SSE; any other status is read whole and raised as LiteLLM's error
pub async fn open_stream(
    setup: StreamSetup<'_>,
    body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
) -> Result<AgentChunkStream, AgentResultError> {
    if !accepted(setup.status) {
        let bytes = read_body(body, MAX_ERROR_BODY_BYTES).await?;
        let Ok(text) = String::from_utf8(bytes) else {
            return Err(AgentResultError::StreamFailedNotText {
                status: setup.status,
            });
        };
        return Err(AgentResultError::StreamFailed {
            status: setup.status,
            body: text,
        });
    }
    Ok(AgentChunkStream::new(body, setup.model, setup.identity))
}

const fn accepted(status: u16) -> bool {
    status == STATUS_OK || status == STATUS_CREATED
}

async fn read_body(
    mut body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    limit: usize,
) -> Result<Vec<u8>, AgentResultError> {
    let mut collected = Vec::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        let Some(total) = collected.len().checked_add(chunk.len()) else {
            return Err(AgentResultError::BodyTooLarge { limit });
        };
        if total > limit {
            return Err(AgentResultError::BodyTooLarge { limit });
        }
        collected.extend_from_slice(&chunk);
    }
    Ok(collected)
}

pub(super) fn empty_body() -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
    empty().boxed()
}
