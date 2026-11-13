use serde_json::Value;
use uuid::Uuid;

use super::error::ResponsesChatStreamError;
use super::records::{ChatChunk, ChunkFactory, PaddingChunk};
use super::text::repr_value;
use super::translate::{ToolCallIndices, translate_responses_chunk_to_openai_stream};
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_to_chat_stream::{DecodedEventLine, decode_responses_chat_event_line};

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedChatLine {
    Padding(PaddingChunk),
    Chunk(ChatChunk),
}

// OpenAiResponsesToChatCompletionStreamIterator: one chat id, one served tier and one index map per stream
pub struct ResponsesChatStream {
    pub json_mode: Option<bool>,
    pub chat_completion_id: Option<String>,
    pub served_service_tier: Option<String>,
    pub tool_call_index_map: ToolCallIndices,
    pub factory: ChunkFactory,
}

// LiteLLM answers with this one iterator for the sync and the async stream, so json_mode just rides along
#[must_use]
pub fn get_model_response_iterator(
    json_mode: Option<bool>,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> ResponsesChatStream {
    ResponsesChatStream {
        json_mode,
        chat_completion_id: None,
        served_service_tier: None,
        tool_call_index_map: ToolCallIndices::new(),
        factory: ChunkFactory { identifier, clock },
    }
}

impl ResponsesChatStream {
    pub fn chunk_parser(&mut self, event: &Value) -> Result<ChatChunk, ResponsesChatStreamError> {
        tracing::debug!(
            "Chat provider: transform_streaming_response called with chunk: {}",
            repr_value(event)
        );
        self.remember_served_service_tier(event);
        let chunk = translate_responses_chunk_to_openai_stream(
            event,
            Some(&mut self.tool_call_index_map),
            self.factory,
        )?;
        let scoped = self.with_stream_scoped_id(chunk);
        Ok(self.with_served_service_tier(scoped))
    }

    pub fn parse_line(
        &mut self,
        bytes: &[u8],
        limits: &ApiLimits,
    ) -> Result<ParsedChatLine, ResponsesChatStreamError> {
        match decode_responses_chat_event_line(bytes, limits)? {
            DecodedEventLine::Padding(padding) => Ok(ParsedChatLine::Padding(padding)),
            DecodedEventLine::Event(event) => Ok(ParsedChatLine::Chunk(self.chunk_parser(&event)?)),
        }
    }

    fn remember_served_service_tier(&mut self, event: &Value) {
        let Some(Value::Object(fields)) = event.get("response") else {
            return;
        };
        if let Some(Value::String(tier)) = fields.get("service_tier")
            && !tier.is_empty()
        {
            self.served_service_tier = Some(tier.clone());
        }
    }

    fn with_stream_scoped_id(&mut self, mut chunk: ChatChunk) -> ChatChunk {
        match &self.chat_completion_id {
            Some(id) => {
                chunk.id.clone_from(id);
                chunk
            }
            None => {
                self.chat_completion_id = Some(chunk.id.clone());
                chunk
            }
        }
    }

    // the terminal chunk's own service_tier, even an empty string, is kept over the remembered one
    fn with_served_service_tier(&self, mut chunk: ChatChunk) -> ChatChunk {
        if let Some(tier) = &self.served_service_tier
            && chunk.service_tier.is_none()
        {
            chunk.service_tier = Some(tier.clone());
        }
        chunk
    }
}
