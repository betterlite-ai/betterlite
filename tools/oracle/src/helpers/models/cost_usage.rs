use betterlite_models::cost_usage::error::CostUsageError;
use betterlite_models::cost_usage::interactions::record::{
    InteractionsUsage, is_interactions_usage_object,
};
use betterlite_models::cost_usage::interactions::transform::transform_interactions_usage;
use betterlite_models::cost_usage::transcription::{
    TranscriptionUsage, transform_transcription_usage,
};
use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) transcription: Vec<TranscriptionUsage>,
    #[serde(default)]
    pub(crate) interactions: Vec<Value>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let transcription: Vec<Value> = input
            .transcription
            .iter()
            .map(|usage| {
                transform_transcription_usage(usage)
                    .map_or(Value::Null, |cost| cost_usage_to_value(&cost))
            })
            .collect();
        let interactions: Vec<Value> = input
            .interactions
            .iter()
            .map(interactions_result)
            .collect::<Result<_, _>>()?;
        Ok(json!({"transcription": transcription, "interactions": interactions}))
    }
}

fn interactions_result(raw: &Value) -> crate::helpers::HelperResult {
    if !raw.is_object() {
        return Ok(json!({"recognized": false, "usage": null}));
    }
    let usage: InteractionsUsage = serde_json::from_value(raw.clone()).map_err(|error| {
        Box::new(HelperError {
            class: "InteractionsDecodeError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    if !is_interactions_usage_object(&usage) {
        return Ok(json!({"recognized": false, "usage": null}));
    }
    let cost = transform_interactions_usage(&usage).map_err(cost_usage_error)?;
    Ok(json!({"recognized": true, "usage": cost_usage_to_value(&cost)}))
}

fn cost_usage_error(error: CostUsageError) -> Box<HelperError> {
    let class = match error {
        CostUsageError::CounterOverflow => "CounterOverflow",
        CostUsageError::ModalityListNotArray => "ModalityListNotArray",
        CostUsageError::ModalityEntryNotObject => "ModalityEntryNotObject",
        CostUsageError::ModalitySumOverflow { .. } => "ModalitySumOverflow",
        CostUsageError::CacheSubtractionOverflow { .. } => "CacheSubtractionOverflow",
        CostUsageError::SearchCountOverflow => "SearchCountOverflow",
        CostUsageError::PromptTotalOverflow => "PromptTotalOverflow",
        CostUsageError::CompletionTotalOverflow => "CompletionTotalOverflow",
        CostUsageError::TotalTokensOverflow => "TotalTokensOverflow",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn cost_usage_to_value(usage: &CostUsage) -> Value {
    let prompt_details = usage.prompt_details.map(|details: PromptDetails| {
        json!({
            "text_tokens": details.text_tokens,
            "audio_tokens": details.audio_tokens,
            "image_tokens": details.image_tokens,
            "video_tokens": details.video_tokens,
            "web_search_requests": details.web_search_requests,
        })
    });
    let completion_details = usage.completion_details.map(|details: CompletionDetails| {
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
        "prompt_details": prompt_details,
        "completion_details": completion_details,
        "cache_read_tokens": usage.cache_read_tokens,
    })
}

mod tests;
