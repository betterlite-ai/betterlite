use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_models::decisions;
use betterlite_models::decisions::HiddenParams;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use crate::api::messages_stream_split::AsyncChunkSource;
use crate::api::messages_stream_split::errors::{
    StreamFailure, error_status_and_message, mid_stream_error_sse_event,
};

use super::driver::{ChatStreamDriver, MID_STREAM_LOG_PREFIX};
use super::sse::{StreamItem, encode_item};

// LiteLLM's AnthropicSSEStream: the byte generator plus the wrapper metadata the proxy bills a stream from
pub struct AnthropicSseStream<S, I> {
    pub stream: ChatStreamDriver<S, I>,
    pub hidden_params: HiddenParams,
    pub closed: bool,
    pub finished: bool,
    pub limits: WireDecodeLimits,
}

impl<S, I> AnthropicSseStream<S, I>
where
    S: AsyncChunkSource + Send,
    I: IdSource,
{
    // LiteLLM's AnthropicSSEStream.__init__: the byte generator, no hidden params yet, and an open view
    #[must_use]
    pub fn new(stream: ChatStreamDriver<S, I>, limits: WireDecodeLimits) -> Self {
        Self {
            stream,
            hidden_params: HiddenParams::new(),
            closed: false,
            finished: false,
            limits,
        }
    }

    // __anext__: the next encoded frame, and the one error event instead of a failure that would end the stream
    pub async fn next_frame(&mut self) -> Option<Vec<u8>> {
        if self.closed || self.finished {
            return None;
        }
        let failure = match self.stream.next_event().await {
            Ok(Some(event)) => match encode_item(StreamItem::Event(event)) {
                Ok(frame) => return Some(frame),
                Err(error) => StreamFailure::Unclassified {
                    text: error.to_string(),
                },
            },
            Ok(None) => {
                self.finished = true;
                return None;
            }
            Err(error) => error.into_stream_failure(),
        };
        self.finished = true;
        tracing::error!(
            kind = "stream_failure",
            "{}{}",
            MID_STREAM_LOG_PREFIX,
            error_status_and_message(&failure).1
        );
        let Ok(frame) = mid_stream_error_sse_event(&failure, self.limits) else {
            tracing::error!(
                kind = "stream_failure",
                "{}the error event exceeds the wire limits",
                MID_STREAM_LOG_PREFIX
            );
            return None;
        };
        Some(frame.into_bytes())
    }

    // aclose closes the byte generator; the provider stream belongs to the wrapper and is not closed here
    pub const fn aclose(&mut self) {
        self.closed = true;
        self.finished = true;
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn chunks(&self) -> Option<&[StreamChunk]> {
        self.stream.state.completion_stream.source.chunks()
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn messages(&self) -> Option<&[decisions::Json]> {
        self.stream.state.completion_stream.source.messages()
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn model(&self) -> &str {
        &self.stream.state.model
    }
}
