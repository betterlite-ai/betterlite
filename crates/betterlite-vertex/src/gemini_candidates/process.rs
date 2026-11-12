use betterlite_models::decisions::Json;
use betterlite_openai::stream::chunk::Logprobs;
use uuid::Uuid;

use crate::gemini_candidates::candidate::Candidate;
use crate::gemini_candidates::choice::{
    Choice, Choices, Delta, Message, Role, SpecificFields, StreamChoice,
};
use crate::gemini_candidates::error::CandidatesError;
use crate::gemini_candidates::finish::check_finish_reason;
use crate::gemini_metadata::annotations::grounding_annotations;
use crate::gemini_metadata::extension::{MetadataExtension, MetadataField};
use crate::gemini_metadata::extract::extract_candidate_metadata;
use crate::gemini_response_parts::content::ThinkingBlock;
use crate::gemini_response_parts::projection::{PartContext, project_parts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Complete,
    Streaming,
}

#[derive(Debug)]
pub struct Processed {
    pub choices: Choices,
    pub metadata: MetadataExtension,
    pub next_tool_call_index: u32,
}

// the tool call index runs on across candidates, and a candidate with neither content nor finishReason adds nothing
pub fn process_candidates(
    candidates: &[Candidate],
    mode: Mode,
    mut context: PartContext,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<Processed, CandidatesError> {
    let mut metadata = MetadataExtension::default();
    let mut complete = Vec::new();
    let mut streaming = Vec::new();
    for (position, candidate) in (0_u32..).zip(candidates) {
        if candidate.content.is_none() && candidate.finish_reason.is_none() {
            continue;
        }
        let (message, logprobs) = assemble(candidate, &mut context, &mut metadata, ids)?;
        let native = candidate.finish_reason.as_deref();
        let finish_reason = check_finish_reason(&message, native).as_str();
        let index = candidate.index.unwrap_or(position);
        match mode {
            Mode::Complete => complete.push(Choice {
                index,
                finish_reason,
                message,
                logprobs,
                native_finish_reason: candidate.finish_reason.clone(),
            }),
            Mode::Streaming => streaming.push(StreamChoice {
                index,
                finish_reason,
                delta: delta_of(message),
                logprobs,
            }),
        }
    }
    Ok(Processed {
        choices: match mode {
            Mode::Complete => Choices::Complete(complete),
            Mode::Streaming => Choices::Streaming(streaming),
        },
        metadata,
        next_tool_call_index: context.cumulative_tool_call_index,
    })
}

fn assemble(
    candidate: &Candidate,
    context: &mut PartContext,
    total: &mut MetadataExtension,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<(Message, Option<Logprobs>), CandidatesError> {
    let extension = extract_candidate_metadata(&Json::Object(candidate.metadata.clone()));
    let parts = candidate
        .content
        .as_ref()
        .and_then(|content| content.parts.as_deref());
    // without parts the projection only carries the logprobs
    let projection = project_parts(
        parts.unwrap_or_default(),
        candidate.logprobs_result.as_ref(),
        *context,
        ids,
    )?;
    context.cumulative_tool_call_index = projection.next_tool_call_index;
    for field in MetadataField::ALL {
        total
            .values_mut(field)
            .extend_from_slice(extension.values(field));
    }
    let has_parts = parts.is_some();
    let annotations = has_parts
        .then(|| grounding_annotations(&extension.grounding_metadata))
        .filter(|found| !found.is_empty());
    let fields = SpecificFields {
        thought_signatures: projection.thought_signatures,
        server_side_tool_invocations: projection.server_side_tool_invocations,
    };
    let has_fields =
        fields.thought_signatures.is_some() || fields.server_side_tool_invocations.is_some();
    let message = Message {
        role: Role::Assistant,
        content: projection.content,
        reasoning_content: projection.reasoning_content,
        thinking_blocks: has_parts.then_some(projection.thinking_blocks),
        tool_calls: projection.tool_calls,
        function_call: projection.function,
        images: has_parts.then_some(projection.images),
        audio: projection.audio,
        annotations,
        provider_specific_fields: has_fields.then_some(fields),
    };
    Ok((message, projection.logprobs))
}

// a streamed choice rebuilds reasoning from the thinking blocks only when the parts gave none
fn delta_of(message: Message) -> Delta {
    let reasoning_content = message.reasoning_content.or_else(|| {
        let texts: Vec<&str> = message
            .thinking_blocks
            .iter()
            .flatten()
            .map(|ThinkingBlock::Thinking { thinking, .. }| thinking.as_str())
            .filter(|text| !text.is_empty())
            .collect();
        (!texts.is_empty()).then(|| texts.join("\n"))
    });
    Delta {
        role: None,
        content: message.content,
        reasoning_content,
        tool_calls: message.tool_calls,
        function_call: message.function_call,
        images: message.images.filter(|images| !images.is_empty()),
        annotations: message.annotations,
        provider_specific_fields: message.provider_specific_fields,
    }
}
