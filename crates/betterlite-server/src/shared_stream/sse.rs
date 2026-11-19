use betterlite_anthropic::api::messages_errors::AnthropicErrorSseFrame;
use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_anthropic::api::messages_errors::frame::anthropic_error_sse_frame;
use betterlite_core::api::messages_stream_restamp::MessagesStreamChunk;
use betterlite_core::api::messages_stream_restamp::restamp::StreamModelRestamper;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use bytes::Bytes;
use futures_util::stream::BoxStream;

use super::call::{SharedCall, StreamAbort};
use super::framing::ChunkFraming;
use super::generator::SharedStreamGenerator;
use super::pings::with_sse_pings;
use super::tail::{
    ANTHROPIC_PING_CHUNK, SSE_STREAM_START_TAIL, advance_sse_tail, seal_open_sse_frame,
};
use crate::sse_data_generator::types::StreamHooks;
use crate::stream_chunks::StreamItem;
use crate::stream_chunks::restamp::should_return_raw_model_name;

// The messages flavor hands the route an Anthropic error frame as its own variant while nothing was written before it
#[derive(Debug)]
pub enum Emitted {
    Frame(Bytes),
    AnthropicError(Box<AnthropicErrorSseFrame>),
}

impl Emitted {
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        match self {
            Self::Frame(bytes) => bytes,
            Self::AnthropicError(frame) => Bytes::from(frame.body),
        }
    }
}

pub struct SseFraming {
    restamper: Option<StreamModelRestamper>,
    tail: Vec<u8>,
    limits: WireDecodeLimits,
}

impl SseFraming {
    #[must_use]
    pub fn new(restamp_model: Option<String>, limits: WireDecodeLimits) -> Self {
        let restamper = restamp_model
            .filter(|model| !model.is_empty())
            .map(StreamModelRestamper::new);
        Self {
            restamper,
            tail: SSE_STREAM_START_TAIL.to_vec(),
            limits,
        }
    }
}

impl ChunkFraming for SseFraming {
    type Out = Emitted;

    fn frame_chunk(&mut self, item: StreamItem) -> Result<Emitted, StreamAbort> {
        let chunk = match item {
            StreamItem::Chunk(chunk) => MessagesStreamChunk::Event(Json::Object(chunk.body)),
            StreamItem::Bytes(bytes) => MessagesStreamChunk::Bytes(bytes.to_vec()),
            StreamItem::Text(text) => MessagesStreamChunk::Text(text),
        };
        let processed = match self.restamper.as_mut() {
            Some(restamper) => restamper.process(&chunk),
            None => chunk,
        };
        let written = match processed {
            MessagesStreamChunk::Event(event) => {
                format!("data: {}\n\n", dumps(&event)).into_bytes()
            }
            MessagesStreamChunk::Bytes(bytes) => bytes,
            MessagesStreamChunk::Text(text) => text.into_bytes(),
        };
        self.tail = advance_sse_tail(&self.tail, &written);
        Ok(Emitted::Frame(Bytes::from(written)))
    }

    fn frame_tail(&mut self) -> Option<Emitted> {
        let restamper = self.restamper.as_mut()?;
        match restamper.flush() {
            MessagesStreamChunk::Bytes(bytes) if !bytes.is_empty() => {
                Some(Emitted::Frame(Bytes::from(bytes)))
            }
            MessagesStreamChunk::Bytes(_)
            | MessagesStreamChunk::Event(_)
            | MessagesStreamChunk::Text(_) => None,
        }
    }

    fn frame_failure(&mut self, error: &ProxyException) -> Result<Emitted, StreamAbort> {
        let status = error.code.parse::<u16>().unwrap_or(500);
        let frame =
            anthropic_error_sse_frame(status, &error.message, self.limits).map_err(|source| {
                tracing::error!("the Anthropic error frame could not be built: {source}");
                StreamAbort {
                    status: 500,
                    detail: source.to_string(),
                }
            })?;
        let seal = seal_open_sse_frame(&self.tail);
        if seal.is_empty() {
            return Ok(Emitted::AnthropicError(Box::new(frame)));
        }
        let mut written = seal.into_bytes();
        written.extend_from_slice(frame.body.as_bytes());
        Ok(Emitted::Frame(Bytes::from(written)))
    }
}

#[must_use]
pub fn restamp_model_for(request_data: &JsonObject, client_model: Option<&str>) -> Option<String> {
    if should_return_raw_model_name(request_data) {
        return None;
    }
    client_model.map(str::to_owned)
}

impl<Key, H> SharedStreamGenerator<Key, H>
where
    Key: Send + Sync + 'static,
    H: StreamHooks<Key> + 'static,
{
    #[must_use]
    pub fn messages_stream(
        &self,
        call: SharedCall<Key>,
        client_model: Option<&str>,
    ) -> BoxStream<'static, Result<Emitted, StreamAbort>> {
        let restamp = restamp_model_for(&call.request_data, client_model);
        let body = self.generate(call, SseFraming::new(restamp, self.limits));
        with_sse_pings(body, self.anthropic_ping.as_ref(), ANTHROPIC_PING_CHUNK)
    }
}
