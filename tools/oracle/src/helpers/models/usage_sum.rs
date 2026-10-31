use std::collections::BTreeMap;

use crate::helpers::HelperError;
use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber,
    usage_sum,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
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
    #[serde(default)]
    pub(crate) prompt_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) completion_tokens: Option<i64>,
    #[serde(default)]
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
    #[serde(default)]
    pub(crate) cached_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) cached_tokens_details: Option<CachedInput>,
    // Accepted for schema symmetry with LiteLLM's input but always dropped: usage_sum never models the cache-creation TTL split.
    #[serde(default, rename = "cache_creation_token_details")]
    pub(crate) _cache_creation_token_details: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachedInput {
    #[serde(default, rename = "text_tokens")]
    pub(crate) text: Option<i64>,
    #[serde(default)]
    pub(crate) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) image_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionInput {
    #[serde(default, rename = "text_tokens")]
    pub(crate) text: Option<i64>,
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
        let result = usage_sum(&usages)
            .map_err(|error| Box::new(HelperError::from(("UsageSumError", error.to_string()))))?;
        usage_value(&result)
    }
}

impl From<(&str, String)> for HelperError {
    fn from((class, message): (&str, String)) -> Self {
        Self {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        }
    }
}

fn convert_entries<T, V, F>(
    source: &BTreeMap<String, T>,
    mut convert: F,
) -> Result<BTreeMap<String, V>, Box<HelperError>>
where
    F: FnMut(&str, &T) -> Result<V, Box<HelperError>>,
{
    let mut result = BTreeMap::new();
    for (name, value) in source {
        result.insert(name.clone(), convert(name, value)?);
    }
    Ok(result)
}

fn number_from_json(name: &str, raw: &Value) -> Result<UsageNumber, Box<HelperError>> {
    if let Some(integer) = raw.as_i64() {
        Ok(UsageNumber::Integer(integer))
    } else if let Some(float) = raw.as_f64() {
        let decimal = Decimal::try_from(float)
            .map_err(|error| Box::new(HelperError::from(("InvalidInput", error.to_string()))))?;
        Ok(UsageNumber::Decimal(decimal))
    } else if let Some(boolean) = raw.as_bool() {
        Ok(UsageNumber::Boolean(boolean))
    } else {
        Err(Box::new(HelperError::from((
            "InvalidInput",
            format!("extra {name} is not numeric"),
        ))))
    }
}

impl UsageInput {
    fn build(&self) -> Result<Usage, Box<HelperError>> {
        let numeric_extras = convert_entries(&self.numeric_extras, number_from_json)?;
        Ok(Usage {
            prompt_tokens: self.prompt_tokens.unwrap_or(0),
            completion_tokens: self.completion_tokens.unwrap_or(0),
            total_tokens: self.total_tokens.unwrap_or(0),
            prompt_tokens_details: self
                .prompt_tokens_details
                .as_ref()
                .map(PromptTokensDetails::from),
            completion_tokens_details: self
                .completion_tokens_details
                .as_ref()
                .map(CompletionTokensDetails::from),
            numeric_extras,
        })
    }
}

impl From<&PromptInput> for PromptTokensDetails {
    fn from(value: &PromptInput) -> Self {
        Self {
            text_tokens: value.text_tokens,
            audio_tokens: value.audio_tokens,
            image_tokens: value.image_tokens,
            video_tokens: value.video_tokens,
            cache_write_tokens: value.cache_write_tokens,
            cache_creation_tokens: value.cache_creation_tokens,
            cached_tokens: value.cached_tokens,
            cached_tokens_details: value
                .cached_tokens_details
                .as_ref()
                .map(CachedTokensDetails::from),
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
        }
    }
}

impl From<&CachedInput> for CachedTokensDetails {
    fn from(value: &CachedInput) -> Self {
        Self {
            text_tokens: value.text,
            audio_tokens: value.audio_tokens,
            image_tokens: value.image_tokens,
        }
    }
}

impl From<&CompletionInput> for CompletionTokensDetails {
    fn from(value: &CompletionInput) -> Self {
        Self {
            text_tokens: value.text,
            reasoning_tokens: value.reasoning_tokens,
            accepted_prediction_tokens: value.accepted_prediction_tokens,
            rejected_prediction_tokens: value.rejected_prediction_tokens,
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
        }
    }
}

fn usage_value(usage: &Usage) -> Result<Value, Box<HelperError>> {
    let prompt_tokens_details = usage
        .prompt_tokens_details
        .as_ref()
        .map(prompt_details_value);
    let completion_tokens_details = usage.completion_tokens_details.as_ref().map(|details| {
        json!({
            "text_tokens": details.text_tokens,
            "reasoning_tokens": details.reasoning_tokens,
            "accepted_prediction_tokens": details.accepted_prediction_tokens,
            "rejected_prediction_tokens": details.rejected_prediction_tokens,
        })
    });
    let numeric_extras = convert_entries(&usage.numeric_extras, |_, value| number_value(value))?;
    Ok(json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": prompt_tokens_details,
        "completion_tokens_details": completion_tokens_details,
        "numeric_extras": numeric_extras,
    }))
}

fn prompt_details_value(details: &PromptTokensDetails) -> Value {
    let cached_tokens_details = details.cached_tokens_details.as_ref().map(|cached| {
        json!({
            "text_tokens": cached.text_tokens,
            "audio_tokens": cached.audio_tokens,
            "image_tokens": cached.image_tokens,
        })
    });
    json!({
        "text_tokens": details.text_tokens,
        "audio_tokens": details.audio_tokens,
        "image_tokens": details.image_tokens,
        "video_tokens": details.video_tokens,
        "cache_write_tokens": details.cache_write_tokens,
        "cache_creation_tokens": details.cache_creation_tokens,
        "cached_tokens": details.cached_tokens,
        "cached_tokens_details": cached_tokens_details,
    })
}

fn number_value(value: &UsageNumber) -> Result<Value, Box<HelperError>> {
    match value {
        UsageNumber::Integer(inner) => Ok(json!(inner)),
        UsageNumber::Decimal(inner) => inner.to_f64().map(|float| json!(float)).ok_or_else(|| {
            Box::new(HelperError::from((
                "InvalidInput",
                format!("decimal extra {inner} has no f64 representation"),
            )))
        }),
        UsageNumber::Boolean(inner) => Ok(json!(inner)),
    }
}
