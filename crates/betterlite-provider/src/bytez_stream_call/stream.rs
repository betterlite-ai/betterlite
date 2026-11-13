use std::str;

use betterlite_redact::secret::Redactor;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::creation::model_response_creator;
use betterlite_streaming::stream_chunk_shaping::decision::return_processed_chunk_logic;
use betterlite_streaming::stream_chunk_shaping::records::{
    ChunkResponseObject, JsonObject, StreamChunk, StreamDelta,
};
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream, Fuse};
use tokio::time::timeout;

use super::STREAMING_TIMEOUT;
use super::error::BytezStreamError;
use super::transport::BytezChunks;

pub type BytezFragments = BoxStream<'static, Result<String, BytezStreamError>>;

// BytezCustomStreamWrapper: the raw transport text plus the shaper whose lifecycle B005 owns
pub struct BytezCustomStream<'request> {
    pub shaper: ChunkShaper,
    pub repeated_chunk_limit: usize,
    pub fragments: BytezFragments,
    pub redactor: &'request Redactor,
}

impl BytezCustomStream<'_> {
    pub fn chunk_creator(
        &mut self,
        fragment: &str,
    ) -> Result<Option<StreamChunk>, BytezStreamError> {
        let model_response = model_response_creator(&mut self.shaper, None, None)
            .map_err(ProviderChunkError::from)
            .map_err(|source| chunk_failure(self, &source))?;
        let mut completion_obj = StreamDelta {
            content: Some(fragment.to_owned()),
            role: None,
            function_call: None,
            tool_calls: None,
            audio: None,
            images: None,
            annotations: None,
            reasoning_content: None,
            thinking_blocks: None,
            provider_specific_fields: None,
            extra: JsonObject::new(),
        };
        let response_obj = ChunkResponseObject::default();
        return_processed_chunk_logic(
            &mut self.shaper,
            &mut completion_obj,
            model_response,
            &response_obj,
            self.repeated_chunk_limit,
        )
        .map_err(ProviderChunkError::from)
        .map_err(|source| chunk_failure(self, &source))
    }
}

fn chunk_failure(stream: &BytezCustomStream<'_>, source: &ProviderChunkError) -> BytezStreamError {
    BytezStreamError::from_chunk(
        source,
        &stream.shaper.model,
        stream.shaper.provider.as_deref().unwrap_or_default(),
        stream.redactor,
    )
}

pub fn text_fragments(chunks: BytezChunks) -> BytezFragments {
    stream::unfold(
        Decoder {
            chunks: chunks.fuse(),
            pending: Vec::new(),
        },
        next_fragment,
    )
    .boxed()
}

struct Decoder {
    chunks: Fuse<BytezChunks>,
    pending: Vec<u8>,
}

async fn next_fragment(
    mut decoder: Decoder,
) -> Option<(Result<String, BytezStreamError>, Decoder)> {
    loop {
        let next = match timeout(STREAMING_TIMEOUT, decoder.chunks.next()).await {
            Ok(Some(next)) => next,
            Ok(None) => return finish(decoder),
            Err(_elapsed) => return Some((Err(BytezStreamError::Timeout), decoder)),
        };
        let bytes = match next {
            Ok(bytes) => bytes,
            Err(source) => return Some((Err(BytezStreamError::Body { source }), decoder)),
        };
        match decode(&mut decoder.pending, &bytes) {
            Ok(text) if text.is_empty() => {}
            Ok(text) => return Some((Ok(text), decoder)),
            Err(error) => return Some((Err(error), decoder)),
        }
    }
}

// httpx flushes its TextDecoder after the last chunk, so bytes still waiting there are never dropped
fn finish(mut decoder: Decoder) -> Option<(Result<String, BytezStreamError>, Decoder)> {
    let Err(source) = str::from_utf8(&decoder.pending) else {
        return None;
    };
    decoder.pending.clear();
    Some((Err(BytezStreamError::InvalidUtf8 { source }), decoder))
}

// httpx's incremental TextDecoder keeps an incomplete multi-byte character for the next fragment
fn decode(pending: &mut Vec<u8>, bytes: &[u8]) -> Result<String, BytezStreamError> {
    pending.extend_from_slice(bytes);
    match str::from_utf8(pending.as_slice()) {
        Ok(text) => {
            let decoded = text.to_owned();
            pending.clear();
            Ok(decoded)
        }
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            let prefix: Vec<u8> = pending.drain(..valid).collect();
            String::from_utf8(prefix).map_err(|source| BytezStreamError::InvalidUtf8 {
                source: source.utf8_error(),
            })
        }
        Err(source) => Err(BytezStreamError::InvalidUtf8 { source }),
    }
}
