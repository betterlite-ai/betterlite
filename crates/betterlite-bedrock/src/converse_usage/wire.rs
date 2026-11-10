use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::converse_usage::error::ConverseUsageError;

// The usage mapping LiteLLM reads; unknown keys are ignored like the Mapping it declares, except totalTokens, whose key presence a default depends on.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ConverseTokenUsage {
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<Json>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<Json>,
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<Json>,
    #[serde(rename = "cacheReadInputTokenCount")]
    pub cache_read_input_token_count: Option<Json>,
    #[serde(rename = "cacheWriteInputTokens")]
    pub cache_write_input_tokens: Option<Json>,
    #[serde(rename = "cacheWriteInputTokenCount")]
    pub cache_write_input_token_count: Option<Json>,
    #[serde(rename = "cacheDetails")]
    pub cache_details: Option<Json>,
    // an absent totalTokens defaults to the prompt-plus-output sum while an explicit null becomes zero, so the map keeps the key as sent
    #[serde(flatten)]
    pub remaining: IndexMap<String, Json>,
}

impl ConverseTokenUsage {
    pub fn decode(value: &Json) -> Result<Self, ConverseUsageError> {
        let text =
            serde_json::to_string(value).map_err(|source| ConverseUsageError::Decode { source })?;
        serde_json::from_str(&text).map_err(|source| ConverseUsageError::Decode { source })
    }
}

// The Converse shape test looks at raw keys, so a null inputTokens or outputTokens still counts as the Converse shape.
#[must_use]
pub fn is_converse_usage_shape(usage: &Json) -> bool {
    let Json::Object(fields) = usage else {
        return false;
    };
    fields.contains_key("inputTokens") || fields.contains_key("outputTokens")
}

#[derive(Debug, Default)]
pub struct ReasoningContentBlock {
    pub reasoning_text: Option<ReasoningText>,
    pub redacted_content: Option<Json>,
}

#[derive(Debug, Default)]
pub struct ReasoningText {
    pub text: Option<String>,
    pub signature: Option<String>,
}

pub fn decode_block(value: &Json) -> Result<ReasoningContentBlock, ConverseUsageError> {
    let Json::Object(fields) = value else {
        return Err(ConverseUsageError::BlockNotObject);
    };
    let reasoning_text = match fields.get("reasoningText") {
        None => None,
        Some(Json::Object(inner)) => Some(ReasoningText {
            text: optional_text(inner.get("text"), "reasoningText.text")?,
            signature: optional_text(inner.get("signature"), "reasoningText.signature")?,
        }),
        Some(_) => {
            return Err(ConverseUsageError::FieldType {
                field: "reasoningText",
            });
        }
    };
    Ok(ReasoningContentBlock {
        reasoning_text,
        redacted_content: fields.get("redactedContent").cloned(),
    })
}

fn optional_text(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, ConverseUsageError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ConverseUsageError::FieldType { field }),
    }
}
