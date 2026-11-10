use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use betterlite_vertex::gemini_usage::metadata::UsageMetadata;
use betterlite_vertex::gemini_usage::usage::{apply_grounding_counters, calculate_usage};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    usage_metadata: Option<UsageMetadata>,
    #[serde(default)]
    candidates: Vec<Json>,
    #[serde(default)]
    grounding_metadata: Vec<Json>,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut usage =
        calculate_usage(input.usage_metadata.as_ref(), &input.candidates).map_err(|error| {
            Box::new(HelperError {
                class: "UsageError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    apply_grounding_counters(&mut usage, &input.grounding_metadata);
    Ok(state(&usage))
}

fn state(usage: &Usage) -> Value {
    let prompt = usage.prompt_tokens_details.as_ref().map(|details| {
        json!({
            "text_tokens": details.text_tokens,
            "audio_tokens": details.audio_tokens,
            "image_tokens": details.image_tokens,
            "video_tokens": details.video_tokens,
            "cached_tokens": details.cached_tokens,
            "tool_use_tokens": details.tool_use_tokens,
            "web_search_requests": details.web_search_requests,
            "google_maps_grounding_requests": details.google_maps_grounding_requests,
        })
    });
    let completion = usage.completion_tokens_details.as_ref().map(|details| {
        json!({
            "text_tokens": details.text_tokens,
            "audio_tokens": details.audio_tokens,
            "image_tokens": details.image_tokens,
            "video_tokens": details.video_tokens,
            "reasoning_tokens": details.reasoning_tokens,
        })
    });
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": prompt,
        "completion_tokens_details": completion,
    })
}
