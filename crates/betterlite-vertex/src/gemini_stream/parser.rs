use super::buffer::FragmentBuffer;
use super::chunk::{Choice, StreamChunk};
use super::embedded::check_embedded_error;
use super::error::StreamError;
use super::{GeminiStreamDecoder, StreamOptions, stream_service_tier};
use crate::gemini_candidates::blocked::stream_block;
use crate::gemini_candidates::candidate::Candidate;
use crate::gemini_candidates::choice::Choices;
use crate::gemini_candidates::process::{Mode, Processed, process_candidates};
use crate::gemini_metadata::extension::MetadataExtension;
use crate::gemini_response::complete::served_model_name;
use crate::gemini_response::decode::{GenerateContentResponse, decode_generate_content};
use crate::gemini_response_parts::projection::PartContext;
use crate::gemini_usage::usage::{apply_grounding_counters, calculate_usage};
use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_models::call_helpers::finish_reason::FinishReason;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use http::header::HeaderMap;
use serde_json::value::RawValue;
use std::str::from_utf8;
use uuid::Uuid;
impl GeminiStreamDecoder {
    #[must_use]
    pub fn new(
        options: StreamOptions,
        headers: &HeaderMap,
        ids: Box<dyn FnMut() -> Uuid + Send>,
    ) -> Self {
        Self {
            options,
            ids,
            service_tier: stream_service_tier(headers),
            accumulated: false,
            buffer: FragmentBuffer::default(),
            cumulative_tool_call_index: 0,
            has_seen_tool_calls: false,
        }
    }
    pub fn push(&mut self, fragment: &[u8]) -> Result<Option<StreamChunk>, StreamError> {
        let text = from_utf8(fragment).map_err(|source| StreamError::Utf8 { source })?;
        if text.is_empty() {
            return Ok(None);
        }
        self.parse_fragment(text)
    }
    pub fn finish(&mut self) -> Result<Option<StreamChunk>, StreamError> {
        if !self.accumulated {
            return Ok(None);
        }
        self.accumulated_chunk("", true)
    }
    fn parse_fragment(&mut self, fragment: &str) -> Result<Option<StreamChunk>, StreamError> {
        let chunk = strip_sse_data_from_chunk(Some(fragment)).unwrap_or(fragment);
        if chunk.is_empty() {
            return Ok(None);
        }
        if self.accumulated {
            return self.accumulated_chunk(chunk, false);
        }
        let trimmed = chunk.trim();
        if let Ok(value) = serde_json::from_str::<Box<RawValue>>(trimmed) {
            return self.chunk_parser(value.get().as_bytes()).map(Some);
        }
        self.accumulated = true;
        self.accumulated_chunk(trimmed, false)
    }
    fn accumulated_chunk(
        &mut self,
        chunk: &str,
        is_final: bool,
    ) -> Result<Option<StreamChunk>, StreamError> {
        let message = strip_sse_data_from_chunk(Some(chunk))
            .unwrap_or(chunk)
            .replace("\n\n", "");
        self.buffer.append(&message)?;
        if !is_final && !self.buffer.could_close {
            return Ok(None);
        }
        loop {
            let Some(value) = self.buffer.pop_next_value() else {
                return Ok(None);
            };
            if value.get().trim_start().starts_with('{') {
                return self.chunk_parser(value.get().as_bytes()).map(Some);
            }
        }
    }
    fn chunk_parser(&mut self, value: &[u8]) -> Result<StreamChunk, StreamError> {
        if value.trim_ascii_start().first() != Some(&b'{') {
            return Err(StreamError::NotAnObject);
        }
        check_embedded_error(value)?;
        let decoded = decode_generate_content(value)?;
        // LiteLLM builds the stream response before it reads the chunk, so the chunk id is drawn first
        let id = match decoded.response_id.clone() {
            Some(response_id) => response_id,
            None => format!("{CHAT_COMPLETION_ID_PREFIX}{}", (self.ids)()),
        };
        let model = served_model_name(decoded.model_version.as_ref()).map(str::to_owned);
        let (choices, metadata) = self.project_choices(&decoded)?;
        let usage = chunk_usage(&decoded, &metadata)?;
        // LiteLLM sets the service tier while it counts usage, so only a chunk with usageMetadata carries it
        let service_tier = usage.as_ref().and_then(|_| self.service_tier.clone());
        Ok(StreamChunk {
            id,
            created: self.options.now,
            model,
            choices,
            usage,
            service_tier,
            metadata,
            traffic_type: decoded.traffic_type,
            is_finished: false,
        })
    }
    fn project_choices(
        &mut self,
        decoded: &GenerateContentResponse,
    ) -> Result<(Vec<Choice>, MetadataExtension), StreamError> {
        let mut choices: Vec<Choice> = stream_block(decoded.prompt_feedback.as_ref())
            .map(|choice| Choice::Projected(Box::new(choice)))
            .into_iter()
            .collect();
        let candidates = decoded.candidates.as_deref().unwrap_or_default();
        if candidates.is_empty() {
            return Ok((choices, MetadataExtension::default()));
        }
        let (projected, metadata) = self.streamed_choices(candidates)?;
        let skipped = choices.is_empty() && projected.is_empty();
        choices.extend(projected);
        if skipped {
            // every candidate here was skipped for lacking finishReason, so its empty delta carries no reason
            for candidate in candidates {
                choices.push(Choice::EmptyDelta {
                    index: candidate.index.unwrap_or(0),
                });
            }
        }
        Ok((choices, metadata))
    }
    fn streamed_choices(
        &mut self,
        candidates: &[Candidate],
    ) -> Result<(Vec<Choice>, MetadataExtension), StreamError> {
        let context = PartContext {
            cumulative_tool_call_index: self.cumulative_tool_call_index,
            call_mode: self.options.call_mode,
            now: self.options.now,
        };
        let Processed {
            choices: built,
            metadata,
            next_tool_call_index,
        } = process_candidates(candidates, Mode::Streaming, context, &mut *self.ids)?;
        self.cumulative_tool_call_index = next_tool_call_index;
        let mut projected = match built {
            Choices::Streaming(found) => found,
            Choices::Complete(_) => Vec::new(),
        };
        if !self.has_seen_tool_calls {
            self.has_seen_tool_calls = projected
                .iter()
                .any(|choice| choice.delta.tool_calls.is_some());
        }
        let rewrite_stop = self.has_seen_tool_calls;
        for choice in &mut projected {
            if rewrite_stop && choice.finish_reason == FinishReason::Stop.as_str() {
                choice.finish_reason = FinishReason::ToolCalls.as_str();
            }
        }
        Ok((
            projected
                .into_iter()
                .map(|choice| Choice::Projected(Box::new(choice)))
                .collect(),
            metadata,
        ))
    }
}
fn chunk_usage(
    decoded: &GenerateContentResponse,
    metadata: &MetadataExtension,
) -> Result<Option<Usage>, StreamError> {
    let Some(reported) = decoded.usage_metadata.as_ref() else {
        return Ok(None);
    };
    let listed: Vec<Json> = decoded
        .candidates
        .iter()
        .flatten()
        .map(|candidate| Json::Object(candidate.metadata.clone()))
        .collect();
    let mut usage = calculate_usage(Some(reported), &listed)?;
    apply_grounding_counters(&mut usage, &metadata.grounding_metadata);
    Ok(Some(usage))
}
