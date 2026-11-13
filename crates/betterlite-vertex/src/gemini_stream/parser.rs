use betterlite_context::provider_message_primitives::CHAT_COMPLETION_ID_PREFIX;
use betterlite_models::call_helpers::finish_reason::FinishReason;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;

use super::GeminiStreamDecoder;
use super::chunk::{Choice, StreamChunk};
use super::embedded::check_embedded_error;
use super::error::StreamError;
use crate::gemini_candidates::blocked::stream_block;
use crate::gemini_candidates::candidate::Candidate;
use crate::gemini_candidates::choice::{Choices, StreamChoice};
use crate::gemini_candidates::process::{Mode, Processed, process_candidates};
use crate::gemini_metadata::extension::MetadataExtension;
use crate::gemini_response::complete::served_model_name;
use crate::gemini_response::decode::{GenerateContentResponse, decode_generate_content};
use crate::gemini_response_parts::projection::PartContext;
use crate::gemini_usage::usage::{apply_grounding_counters, calculate_usage};

impl GeminiStreamDecoder {
    pub(super) fn chunk_parser(&mut self, value: &[u8]) -> Result<StreamChunk, StreamError> {
        if value.trim_ascii_start().first() != Some(&b'{') {
            return Err(StreamError::NotAnObject);
        }
        check_embedded_error(value)?;
        let decoded = decode_generate_content(value)?;
        // LiteLLM builds the stream response before it reads the chunk, so the chunk id is drawn first
        let id = match decoded.response_id {
            Some(response_id) => response_id,
            None => format!("{CHAT_COMPLETION_ID_PREFIX}{}", self.ids.next_id()),
        };
        let model = served_model_name(decoded.model_version.as_ref()).map(str::to_owned);
        let (choices, metadata) = self.project_choices(&decoded)?;
        let usage = chunk_usage(&decoded, &metadata)?;
        Ok(StreamChunk {
            id,
            created: self.options.now,
            model,
            choices,
            usage,
            service_tier: self.service_tier.clone(),
            metadata,
            traffic_type: decoded.traffic_type,
            is_finished: false,
        })
    }

    fn project_choices(
        &mut self,
        decoded: &GenerateContentResponse,
    ) -> Result<(Vec<Choice>, MetadataExtension), StreamError> {
        let mut choices = Vec::new();
        if let Some(blocked) = stream_block(decoded.prompt_feedback.as_ref()) {
            choices.push(Choice::Projected(blocked));
        }
        let candidates = decoded.candidates.as_deref().unwrap_or_default();
        if candidates.is_empty() {
            return Ok((choices, MetadataExtension::default()));
        }
        let (projected, metadata) = self.streamed_choices(candidates)?;
        let skipped = projected.is_empty();
        choices.extend(projected);
        if skipped {
            // every candidate was skipped for lacking finishReason, so the empty delta carries no reason
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
        let processed = {
            let mut ids = || self.ids.next_id();
            process_candidates(
                candidates,
                Mode::Streaming,
                PartContext {
                    cumulative_tool_call_index: self.cumulative_tool_call_index,
                    call_mode: self.options.call_mode,
                    now: self.options.now,
                },
                &mut ids,
            )?
        };
        let Processed {
            choices: built,
            metadata,
            next_tool_call_index,
        } = processed;
        self.cumulative_tool_call_index = next_tool_call_index;
        let mut projected = match built {
            Choices::Streaming(found) => found,
            Choices::Complete(_) => Vec::new(),
        };
        self.apply_tool_call_tracking(&mut projected);
        Ok((
            projected.into_iter().map(Choice::Projected).collect(),
            metadata,
        ))
    }

    fn apply_tool_call_tracking(&mut self, projected: &mut [StreamChoice]) {
        if !self.has_seen_tool_calls {
            self.has_seen_tool_calls = projected
                .iter()
                .any(|choice| choice.delta.tool_calls.is_some());
        }
        if self.has_seen_tool_calls {
            for choice in projected {
                if choice.finish_reason == FinishReason::Stop.as_str() {
                    choice.finish_reason = FinishReason::ToolCalls.as_str();
                }
            }
        }
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
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|candidate| Json::Object(candidate.metadata.clone()))
        .collect();
    let mut usage = calculate_usage(Some(reported), &listed)?;
    apply_grounding_counters(&mut usage, &metadata.grounding_metadata);
    Ok(Some(usage))
}
