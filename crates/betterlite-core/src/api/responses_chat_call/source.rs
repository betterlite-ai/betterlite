use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::sdk::{ByteStream, SdkError};
use betterlite_openai::text_completion_transport::stream::{
    ProviderStream, PumpError, StreamSetup, provider_stream,
};
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamOptions};
use futures_util::StreamExt as _;
use thiserror::Error;
use uuid::Uuid;

use crate::api::chat_response_stream::chunk::wrapper_view;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;

use super::completions::CompletionBody;

// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default
const REPEATED_CHUNK_LIMIT: usize = 100;

// What CustomStreamWrapper is built with when handler.py hands its stream to the Responses iterator
pub struct ChunkSourceConfig {
    pub provider: String,
    pub model: String,
    pub stream_options: Option<StreamOptions>,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

// What the shared pump can meet while it reads B014's body, kept typed so the caller can name each cause
#[derive(Debug, Error)]
enum ChunkError {
    #[error(transparent)]
    Chunk(ProviderChunkError),
    #[error(transparent)]
    Usage(serde_json::Error),
    #[error(transparent)]
    Sdk(Box<SdkError>),
}

// B014's SSE bytes as B924 reads them: the text transport's one pump, one ModelResponseStream per item
pub struct ResponsesChunkSource {
    chunks: ProviderStream<ChunkError>,
}

impl ResponsesChunkSource {
    #[must_use]
    pub fn new(config: ChunkSourceConfig, body: CompletionBody) -> Self {
        // the pump reads the OpenAI SDK's byte error, so a body failure enters it as the SDK's own text
        let bytes: ByteStream = Box::pin(body.0.map(|read| {
            read.map_err(|source| SdkError::Other {
                message: source.to_string(),
            })
        }));
        let chunks = provider_stream(
            &config.provider,
            bytes,
            StreamSetup {
                model: config.model,
                stream_options: config.stream_options,
                merge_reasoning_in_choices: false,
                is_function_call: false,
                base_hidden_params: JsonObject::new(),
                repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
                now: config.now,
                next_id: config.next_id,
            },
            |error| match error {
                PumpError::Chunk(source) => ChunkError::Chunk(source),
                PumpError::Usage(source) => ChunkError::Usage(source),
                PumpError::Sdk(source) => ChunkError::Sdk(source),
            },
        );
        Self { chunks }
    }
}

#[async_trait]
impl AsyncChunkStream for ResponsesChunkSource {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        let Some(item) = self.chunks.next().await else {
            return Ok(None);
        };
        let chunk = item.map_err(|error| StreamFailure::Unclassified {
            text: error.to_string(),
        })?;
        wrapper_view(&chunk)
            .map(Some)
            .map_err(|error| StreamFailure::Unclassified {
                text: error.to_string(),
            })
    }
}
