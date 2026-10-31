use std::collections::BTreeMap;

use crate::helpers::HelperError;
use betterlite_models::usage_sum::{
    CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber, usage_sum,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) usages: Vec<UsageInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UsageInput {
    pub(crate) prompt_tokens: Option<i64>,
    pub(crate) completion_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptInput>,
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<CompletionInput>,
    #[serde(default)]
    pub(crate) numeric_extras: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptInput {
    #[serde(default)]
    pub(crate) text_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) image_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) video_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) cache_write_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) cache_creation_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionInput {
    #[serde(default)]
    pub(crate) text_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) reasoning_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) accepted_prediction_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) rejected_prediction_tokens: Option<i64>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let usages = input
            .usages
            .iter()
            .map(UsageInput::build)
            .collect::<Result<Vec<_>, _>>()?;
        let result = usage_sum(&usages).map_err(|error| {
            Box::new(HelperError {
                class: "UsageSumError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        Ok(usage_value(&result))
    }
}

impl UsageInput {
    fn build(&self) -> Result<Usage, Box<HelperError>> {
        let numeric_extras = self
            .numeric_extras
            .iter()
            .map(|(name, value)| {
                let number = if let Some(value) = value.as_i64() {
                    UsageNumber::Integer(value)
                } else if let Some(value) = value.as_f64() {
                    UsageNumber::Decimal(
                        Decimal::try_from(value).map_err(|error| invalid(error.to_string()))?,
                    )
                } else if let Some(value) = value.as_bool() {
                    UsageNumber::Boolean(value)
                } else {
                    return Err(invalid(format!("extra {name} is not numeric")));
                };
                Ok((name.clone(), number))
            })
            .collect::<Result<_, _>>()?;
        Ok(Usage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            prompt_tokens_details: self.prompt_tokens_details.as_ref().map(PromptInput::build),
            completion_tokens_details: self
                .completion_tokens_details
                .as_ref()
                .map(CompletionInput::build),
            numeric_extras,
        })
    }
}

impl PromptInput {
    fn build(&self) -> PromptTokensDetails {
        let result = PromptTokensDetails {
            text_tokens: self.text_tokens,
            audio_tokens: self.audio_tokens,
            image_tokens: self.image_tokens,
            video_tokens: self.video_tokens,
            cache_write_tokens: self.cache_write_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            cached_tokens_details: None,
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
        };
        result
    }
}

impl CompletionInput {
    fn build(&self) -> CompletionTokensDetails {
        let result = CompletionTokensDetails {
            text_tokens: self.text_tokens,
            reasoning_tokens: self.reasoning_tokens,
            accepted_prediction_tokens: self.accepted_prediction_tokens,
            rejected_prediction_tokens: self.rejected_prediction_tokens,
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
        };
        result
    }
}

fn usage_value(usage: &Usage) -> Value {
    let result = json!({"prompt_tokens": usage.prompt_tokens, "completion_tokens": usage.completion_tokens, "total_tokens": usage.total_tokens, "prompt_tokens_details": usage.prompt_tokens_details.as_ref().map(|details| json!({"text_tokens": details.text_tokens, "audio_tokens": details.audio_tokens, "image_tokens": details.image_tokens, "video_tokens": details.video_tokens, "cache_write_tokens": details.cache_write_tokens, "cache_creation_tokens": details.cache_creation_tokens})), "completion_tokens_details": usage.completion_tokens_details.as_ref().map(|details| json!({"text_tokens": details.text_tokens, "reasoning_tokens": details.reasoning_tokens, "accepted_prediction_tokens": details.accepted_prediction_tokens, "rejected_prediction_tokens": details.rejected_prediction_tokens})), "numeric_extras": usage.numeric_extras.iter().map(|(name, value)| (name.clone(), match value { UsageNumber::Integer(value) => json!(value), UsageNumber::Decimal(value) => json!(value.to_string()), UsageNumber::Boolean(value) => json!(value) })).collect::<BTreeMap<_, _>>() });
    result
}

fn invalid(message: String) -> Box<HelperError> {
    let error = HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}
