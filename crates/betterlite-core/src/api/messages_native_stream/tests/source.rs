#![cfg(test)]

use std::collections::VecDeque;

use async_trait::async_trait;

use super::super::MessagesChunkSource as _;
use super::support::{ProviderError, frame};
use crate::api::messages_sse_events::stream::{
    MessagesProviderStream, MessagesStreamHiddenParams, MessagesStreamingResponse,
};
use crate::api::messages_stream_logging::SseChunk;

struct Provider {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl MessagesProviderStream for Provider {
    type Error = ProviderError;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ProviderError> {
        Ok(self.chunks.pop_front())
    }
}

// The wrapper keeps the default next_sse_chunk, so a provider chunk reaches the relay as raw bytes
#[tokio::test]
async fn a_provider_stream_is_wrapped_as_raw_bytes() {
    let provider = Provider {
        chunks: VecDeque::from([frame("data: one\n\n")]),
    };
    let mut response = MessagesStreamingResponse {
        completion_stream: provider,
        hidden_params: MessagesStreamHiddenParams::default(),
    };
    match response.next_sse_chunk().await {
        Ok(Some(SseChunk::Bytes(bytes))) => assert_eq!(bytes, frame("data: one\n\n")),
        Ok(Some(SseChunk::Mapping(_))) => panic!("a provider chunk stays raw bytes"),
        Ok(None) => panic!("the provider still had a chunk"),
        Err(error) => panic!("the provider yields, got {error}"),
    }
    assert!(matches!(response.next_sse_chunk().await, Ok(None)));
}
