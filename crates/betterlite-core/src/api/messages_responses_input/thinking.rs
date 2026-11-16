use betterlite_prompts::reasoning_fields::encrypted_signature::encrypted_reasoning_signature;
use serde::{Deserialize, Serialize};
use serde_with::{As, DefaultOnNull};

const SUMMARY_PART_SEPARATOR: &str = "\n\n";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SummaryPart {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingBlock {
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
}

fn joined_summary(summary: &[SummaryPart]) -> String {
    summary
        .iter()
        .map(|part| part.text.as_str())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(SUMMARY_PART_SEPARATOR)
}

// The encrypted reasoning rides the block's opaque field, so a summary-less item becomes a redacted block.
#[must_use]
pub fn thinking_block(
    summary: &[SummaryPart],
    encrypted_content: Option<&str>,
) -> Option<ThinkingBlock> {
    let text = joined_summary(summary);
    let Some(encrypted) = encrypted_content.filter(|content| !content.is_empty()) else {
        if text.is_empty() {
            return None;
        }
        return Some(ThinkingBlock::Thinking {
            thinking: text,
            signature: None,
        });
    };
    let signature = encrypted_reasoning_signature(encrypted);
    if text.is_empty() {
        return Some(ThinkingBlock::RedactedThinking { data: signature });
    }
    Some(ThinkingBlock::Thinking {
        thinking: text,
        signature: Some(signature),
    })
}
