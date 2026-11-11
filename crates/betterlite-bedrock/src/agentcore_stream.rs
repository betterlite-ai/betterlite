pub mod adapter;
pub mod line;
pub mod transport;

use std::collections::VecDeque;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};
use futures_util::StreamExt as _;
use http::HeaderMap;
use thiserror::Error;
use uuid::Uuid;

use crate::agentcore_response::error::{AgentCoreResponseError, BedrockFailure};
use crate::agentcore_stream::line::{LineError, LineEvent};
use crate::agentcore_stream::transport::{Chunks, StreamHandler};
use crate::bedrock_settings::HeaderError;

pub const PROVIDER: &str = "bedrock";
pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const FIRST_RESPONSE: &str = "first stream response received";

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

pub struct UuidV4;

impl IdSource for UuidV4 {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

pub struct StreamRequest<'request> {
    pub model: &'request str,
    pub api_base: &'request str,
    pub headers: HeaderMap,
    pub data: &'request Json,
    pub signed_json_body: Option<&'request [u8]>,
    pub messages: &'request Json,
}

pub struct StreamPostCall<'call> {
    pub input: &'call Json,
    pub api_key: &'call str,
    pub original_response: &'call str,
    pub complete_input_dict: &'call Json,
}

pub trait StreamLogger: Send + Sync {
    fn post_call(&self, call: StreamPostCall<'_>);
}

pub struct StreamDeps<'deps> {
    pub supplied: Option<Arc<dyn StreamHandler>>,
    pub default: Arc<dyn StreamHandler>,
    pub ids: Arc<dyn IdSource>,
    pub logger: &'deps dyn StreamLogger,
}

#[derive(Debug, Error)]
pub enum AgentCoreStreamError {
    #[error("the AgentCore request failed")]
    Transport(#[source] TransportError),
    #[error("the AgentCore response body could not be read")]
    Body(#[source] HttpError),
    #[error("the AgentCore endpoint is not a URL")]
    Url(#[source] url::ParseError),
    #[error("the AgentCore request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the AgentCore stream line is invalid: {0}")]
    Line(#[source] LineError),
    #[error("the AgentCore JSON reply could not be projected")]
    Reply(#[source] AgentCoreResponseError),
    #[error("bedrock provider error ({status})")]
    Provider {
        status: u16,
        #[source]
        cause: BedrockFailure,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

pub struct AgentCoreStream {
    pub provider: &'static str,
    pub model: String,
    pub ids: Arc<dyn IdSource>,
    pub chunks: Option<Chunks>,
    pub buffer: Vec<u8>,
    pub pending: VecDeque<Chunk>,
    pub failure: Option<AgentCoreStreamError>,
}

impl AgentCoreStream {
    #[must_use]
    pub fn ready(model: &str, ids: Arc<dyn IdSource>, content: &str) -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(chunk_for(
            model,
            ids.as_ref(),
            LineEvent::Content(content.to_owned()),
        ));
        pending.push_back(chunk_for(model, ids.as_ref(), LineEvent::MessageStop));
        Self {
            provider: PROVIDER,
            model: model.to_owned(),
            ids,
            chunks: None,
            buffer: Vec::new(),
            pending,
            failure: None,
        }
    }

    pub async fn next_chunk(&mut self) -> Result<Option<Chunk>, AgentCoreStreamError> {
        loop {
            if let Some(chunk) = self.pending.pop_front() {
                return Ok(Some(chunk));
            }
            if let Some(error) = self.failure.take() {
                return Err(error);
            }
            let Some(chunks) = self.chunks.as_mut() else {
                return Ok(None);
            };
            let mut events = Vec::new();
            let failure = match chunks.next().await {
                Some(Ok(bytes)) => {
                    line::decode_agentcore_line(&mut self.buffer, &bytes, &mut events)
                        .err()
                        .map(AgentCoreStreamError::Line)
                }
                Some(Err(source)) => Some(AgentCoreStreamError::Body(source)),
                None => {
                    self.chunks = None;
                    None
                }
            };
            if failure.is_some() {
                self.chunks = None;
                self.failure = failure;
            }
            for event in events {
                self.pending
                    .push_back(chunk_for(&self.model, self.ids.as_ref(), event));
            }
        }
    }
}

fn chunk_for(model: &str, ids: &dyn IdSource, event: LineEvent) -> Chunk {
    let (delta, finish_reason, usage) = match event {
        LineEvent::Content(text) => (
            Delta {
                role: Some("assistant".to_owned()),
                content: Some(text),
                refusal: None,
                tool_calls: None,
                function_call: None,
            },
            None,
            None,
        ),
        LineEvent::Usage(usage) => (
            Delta::default(),
            Some("stop".to_owned()),
            Some(usage.to_chat_usage()),
        ),
        LineEvent::MessageStop => (Delta::default(), Some("stop".to_owned()), None),
    };
    Chunk {
        id: format!("chatcmpl-{}", ids.next_id()),
        object: CHUNK_OBJECT.to_owned(),
        created: 0,
        model: model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta,
            finish_reason,
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
        service_tier: None,
    }
}

mod tests;
