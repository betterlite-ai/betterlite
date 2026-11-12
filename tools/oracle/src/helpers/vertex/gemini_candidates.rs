use betterlite_vertex::gemini_candidates::blocked::{complete_block, stream_block};
use betterlite_vertex::gemini_candidates::candidate::{Candidate, PromptFeedback};
use betterlite_vertex::gemini_candidates::process::{Mode, process_candidates};
use betterlite_vertex::gemini_metadata::extension::MetadataExtension;
use betterlite_vertex::gemini_response_parts::projection::{CallMode, PartContext};
use betterlite_vertex::gemini_usage::metadata::UsageMetadata;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    candidates: Option<Vec<Candidate>>,
    prompt_feedback: Option<PromptFeedback>,
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    streaming: bool,
    legacy_function: bool,
    cumulative_tool_call_idx: u32,
    now: i64,
    generated_id: String,
    body: Body,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let uuid = Uuid::parse_str(&input.generated_id).or_value_error()?;
        let context = PartContext {
            cumulative_tool_call_index: input.cumulative_tool_call_idx,
            call_mode: if input.legacy_function {
                CallMode::LegacyFunction
            } else {
                CallMode::Tools
            },
            now: input.now,
        };
        if input.streaming {
            streamed(&input.body, context, uuid)
        } else {
            complete(&input.body, context, uuid)
        }
    }
}

fn complete(body: &Body, context: PartContext, uuid: Uuid) -> HelperResult {
    let candidates = body.candidates.as_deref().unwrap_or_default();
    let blocked = complete_block(
        body.prompt_feedback.as_ref(),
        candidates,
        body.usage_metadata.as_ref(),
    )
    .or_value_error()?;
    if let Some(found) = blocked {
        return Ok(json!({
            "choices": [found.choice],
            "usage": {
                "prompt_tokens": found.usage.prompt_tokens,
                "completion_tokens": found.usage.completion_tokens,
                "total_tokens": found.usage.total_tokens,
                "prompt_tokens_details": found.usage.prompt_tokens_details.map(|_| "present"),
                "completion_tokens_details": found.usage.completion_tokens_details.map(|_| "present"),
            },
            "metadata": null,
        }));
    }
    let processed =
        process_candidates(candidates, Mode::Complete, context, &mut || uuid).or_value_error()?;
    Ok(
        json!({"choices": processed.choices, "usage": null, "metadata": metadata(&processed.metadata)}),
    )
}

fn streamed(body: &Body, context: PartContext, uuid: Uuid) -> HelperResult {
    let block = stream_block(body.prompt_feedback.as_ref());
    let candidates = body.candidates.as_deref().unwrap_or_default();
    let Some(choice) = block else {
        let processed = process_candidates(candidates, Mode::Streaming, context, &mut || uuid)
            .or_value_error()?;
        return Ok(json!({
            "choices": processed.choices,
            "metadata": metadata(&processed.metadata),
            "next_tool_call_index": processed.next_tool_call_index,
        }));
    };
    Ok(
        json!({"choices": [choice], "metadata": null, "next_tool_call_index": context.cumulative_tool_call_index}),
    )
}

fn metadata(extension: &MetadataExtension) -> Value {
    json!({
        "grounding_metadata": extension.grounding_metadata,
        "url_context_metadata": extension.url_context_metadata,
        "safety_ratings": extension.safety_ratings,
        "citation_metadata": extension.citation_metadata,
    })
}
