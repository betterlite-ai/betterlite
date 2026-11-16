use std::error::Error as StdError;
use std::future::Future;

use serde_json::value::RawValue;

use crate::presidio_response::wire::ChatModelResponse;

// The helpers other tasks own, passed in so the stream hooks call them by LiteLLM's names without copying them
pub trait StreamAssembly: Sync {
    type Error: StdError + Send + 'static;

    // is_anthropic_sse_stream((chunk,)), B3669
    fn is_anthropic_sse_stream(&self, chunk: &[u8]) -> bool;

    // stream_chunk_builder, B2227: None when it does not build a chat ModelResponse
    fn stream_chunk_builder(
        &self,
        chunks: &[Box<RawValue>],
        messages: Option<&RawValue>,
    ) -> impl Future<Output = Result<Option<ChatModelResponse>, Self::Error>> + Send;

    // convert_to_streaming, B2227
    fn convert_to_streaming(
        &self,
        response: &ChatModelResponse,
    ) -> Result<Box<RawValue>, Self::Error>;

    // assemble_anthropic_sse_stream(chunks, restore_identity=True), B3669: None when the frames do not assemble
    fn assemble_anthropic_sse_stream(
        &self,
        chunks: &[Vec<u8>],
    ) -> impl Future<Output = Option<ChatModelResponse>> + Send;

    // model_response_text, B3669
    fn model_response_text(&self, response: &ChatModelResponse) -> String;

    // anthropic_sse_chunks, B3669
    fn anthropic_sse_chunks(
        &self,
        response: &ChatModelResponse,
    ) -> Result<Vec<Vec<u8>>, Self::Error>;
}
