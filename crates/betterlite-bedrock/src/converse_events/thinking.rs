use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::converse_events::error::ConverseEventError;
use crate::converse_events::response::ThinkingBlock;
use crate::converse_events::wire;

// text wins over signature, signature over redactedContent, and a block with none of the three carries nothing.
pub fn translate_thinking_blocks(
    block: &IndexMap<String, Json>,
) -> Result<Vec<ThinkingBlock>, ConverseEventError> {
    if let Some(text) = block.get("text") {
        return Ok(vec![ThinkingBlock::Thinking {
            signature: None,
            thinking: wire::text(text, "reasoningContent.text")?.to_owned(),
        }]);
    }
    if let Some(signature) = block.get("signature") {
        return Ok(vec![ThinkingBlock::Thinking {
            signature: Some(wire::text(signature, "reasoningContent.signature")?.to_owned()),
            thinking: String::new(),
        }]);
    }
    if let Some(data) = block.get("redactedContent") {
        return Ok(vec![ThinkingBlock::Redacted {
            data: wire::text(data, "reasoningContent.redactedContent")?.to_owned(),
        }]);
    }
    Ok(Vec::new())
}
