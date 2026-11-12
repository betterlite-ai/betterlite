use betterlite_vertex::gemini_candidates::blocked::{complete_block, stream_block};
use betterlite_vertex::gemini_candidates::candidate::{Candidate, PromptFeedback};
use betterlite_vertex::gemini_candidates::process::{Mode, process_candidates};
use betterlite_vertex::gemini_metadata::extension::MetadataExtension;
use betterlite_vertex::gemini_response_parts::projection::{CallMode, PartContext};
use betterlite_vertex::gemini_usage::metadata::UsageMetadata;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

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
        if let Some(answer) = blocked(input, context.cumulative_tool_call_index)? {
            return Ok(answer);
        }
        let candidates = input.body.candidates.as_deref().unwrap_or_default();
        let mode = if input.streaming {
            Mode::Streaming
        } else {
            Mode::Complete
        };
        let kept = process_candidates(candidates, mode, context, &mut || uuid).or_value_error()?;
        let extension = metadata(&kept.metadata);
        Ok(if input.streaming {
            json!({"choices": kept.choices, "metadata": extension, "next_tool_call_index": kept.next_tool_call_index})
        } else {
            json!({"choices": kept.choices, "usage": null, "metadata": extension})
        })
    }
}

fn blocked(input: &Input, next_tool_call_index: u32) -> Result<Option<Value>, Box<HelperError>> {
    let body = &input.body;
    if input.streaming {
        let flagged = stream_block(body.prompt_feedback.as_ref());
        return Ok(flagged.map(|choice| {
            json!({"choices": [choice], "metadata": null, "next_tool_call_index": next_tool_call_index})
        }));
    }
    let candidates = body.candidates.as_deref().unwrap_or_default();
    let found = complete_block(
        body.prompt_feedback.as_ref(),
        candidates,
        body.usage_metadata.as_ref(),
    )
    .or_value_error()?;
    Ok(found.map(|block| {
        json!({
            "choices": [block.choice],
            "usage": {
                "prompt_tokens": block.usage.prompt_tokens,
                "completion_tokens": block.usage.completion_tokens,
                "total_tokens": block.usage.total_tokens,
                "prompt_tokens_details": block.usage.prompt_tokens_details.map(|_| "present"),
                "completion_tokens_details": block.usage.completion_tokens_details.map(|_| "present"),
            },
            "metadata": null,
        })
    }))
}

fn metadata(extension: &MetadataExtension) -> Value {
    json!({
        "grounding_metadata": extension.grounding_metadata,
        "url_context_metadata": extension.url_context_metadata,
        "safety_ratings": extension.safety_ratings,
        "citation_metadata": extension.citation_metadata,
    })
}
