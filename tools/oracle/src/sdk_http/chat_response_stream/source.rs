use std::collections::VecDeque;
use std::fmt::Display;

use async_trait::async_trait;
use betterlite_core::api::chat_response_stream::chunk::wrapper_view;
use betterlite_core::api::chat_response_stream::source::{AsyncChunkStream, ChunkStream};
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_models::decisions::Json;
use betterlite_streaming::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue};
use betterlite_streaming::stream_chunk_shaping::{ChunkShaper, tokens::finish_reason_handler};
use uuid::Uuid;

const DONE: &[u8] = b"[DONE]";

// The mock's SSE events decode through B004's chunk creator, one wrapper-view chunk per pull
pub(super) struct PayloadSource {
    payloads: VecDeque<Vec<u8>>,
    shaper: ChunkShaper,
    state: DecodeState,
}

impl PayloadSource {
    pub(super) fn new(model: String, payloads: VecDeque<Vec<u8>>) -> Self {
        let mut shaper = ChunkShaper::new(
            model.clone(),
            Some("openai".to_owned()),
            false,
            JsonObject::new(),
        );
        shaper.now = || StreamInstant {
            seconds: 0,
            timestamp: 0.0,
        };
        shaper.next_id = Uuid::nil;
        Self {
            payloads,
            shaper,
            state: DecodeState {
                model,
                kind: ProviderKind::Builtin,
                received_finish_reason: None,
                intermittent_finish_reason: None,
                sent_first_chunk: false,
                complete_response: String::new(),
                completion_stream: None,
                options: RequestOptions {
                    send_stream_usage: true,
                    is_function_call: false,
                },
                repeated_chunk_limit: 100,
                next_id: Uuid::nil,
            },
        }
    }

    // A null payload reaches the driver as a null chunk; the sentinel ends the stream before the parser does
    fn decode_next(&mut self) -> Result<Option<Json>, StreamFailure> {
        while let Some(payload) = self.payloads.pop_front() {
            if payload == DONE {
                self.payloads.clear();
                break;
            }
            let value: JsonValue = serde_json::from_slice(&payload).map_err(failure)?;
            if value == JsonValue::Null {
                return Ok(Some(Json::Null));
            }
            let Some(chunk) = chunk_creator(
                &mut self.shaper,
                &mut self.state,
                &ProviderChunk::Json(Box::new(value)),
            )
            .map_err(failure)?
            else {
                continue;
            };
            return wrapper_view(&chunk).map(Some).map_err(failure);
        }
        // The wrapper invents a finish chunk when the provider's stream ends without one
        if !self.shaper.progress.sent_last_chunk {
            self.shaper.progress.sent_last_chunk = true;
            let chunk = finish_reason_handler(&mut self.shaper);
            return wrapper_view(&chunk).map(Some).map_err(failure);
        }
        Ok(None)
    }
}

// The decode path's errors all surface as the same unclassified stream failure
fn failure<Error>(error: Error) -> StreamFailure
where
    Error: Display,
{
    StreamFailure::Unclassified {
        text: error.to_string(),
    }
}

impl ChunkStream for PayloadSource {
    fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        self.decode_next()
    }
}

#[async_trait]
impl AsyncChunkStream for PayloadSource {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        self.decode_next()
    }
}
