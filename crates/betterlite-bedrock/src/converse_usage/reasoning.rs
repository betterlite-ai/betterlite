use betterlite_anthropic::usage::thinking_tokens_from_usage;
use betterlite_models::decisions::Json;
use serde::Serialize;

use crate::converse_usage::error::ConverseUsageError;
use crate::converse_usage::wire::decode_block;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ThinkingBlock {
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    #[serde(rename = "redacted_thinking")]
    Redacted { data: Json },
}

// Every reasoningText.text concatenates with no separator, and a reasoningText block without text fails the concatenation.
pub fn transform_reasoning_content(blocks: &[Json]) -> Result<String, ConverseUsageError> {
    let mut content = String::new();
    for value in blocks {
        let block = decode_block(value)?;
        if let Some(reasoning) = block.reasoning_text {
            let Some(text) = reasoning.text else {
                return Err(ConverseUsageError::ReasoningTextMissing);
            };
            content.push_str(&text);
        }
    }
    Ok(content)
}

// reasoningText wins over redactedContent, an empty text or signature stays, and a block with neither member is ignored.
pub fn transform_thinking_blocks(
    blocks: &[Json],
) -> Result<Vec<ThinkingBlock>, ConverseUsageError> {
    let mut result = Vec::new();
    for value in blocks {
        let block = decode_block(value)?;
        match (block.reasoning_text, block.redacted_content) {
            (Some(reasoning), _) => result.push(ThinkingBlock::Thinking {
                thinking: reasoning.text,
                signature: reasoning.signature,
            }),
            (None, Some(data)) => result.push(ThinkingBlock::Redacted { data }),
            (None, None) => {}
        }
    }
    Ok(result)
}

// additionalModelResponseFields carries the provider thinking count only when both it and its usage are mappings.
#[must_use]
pub fn thinking_tokens_from_fields(additional_fields: &Json) -> Option<i64> {
    let Json::Object(fields) = additional_fields else {
        return None;
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return None;
    };
    thinking_tokens_from_usage(&Json::Object(usage.clone()))
}
