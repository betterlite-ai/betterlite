use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::api::messages_context_clear::constants::COMPACT_EDIT_TYPE;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct AppliedEdit {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub edit_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_tool_uses: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_thinking_turns: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_input_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_output_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionKind {
    Compaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompactionBlock {
    #[serde(rename = "type")]
    pub kind: CompactionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageKind {
    Compaction,
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct UsageIteration {
    #[serde(rename = "type")]
    pub kind: UsageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolyfillResult {
    pub messages: Vec<Json>,
    pub system: Option<Json>,
    pub applied_edits: Vec<AppliedEdit>,
    pub compaction_block: Option<CompactionBlock>,
    pub iterations_usage: Option<Vec<UsageIteration>>,
}

impl PolyfillResult {
    #[must_use]
    pub fn applied_edits_for_response(&self) -> Option<Vec<AppliedEdit>> {
        let visible: Vec<AppliedEdit> = self
            .applied_edits
            .iter()
            .filter(|edit| {
                if edit.edit_type.as_deref() != Some(COMPACT_EDIT_TYPE) {
                    return true;
                }
                self.compaction_block.is_some()
                    || edit.error.as_deref().is_some_and(|text| !text.is_empty())
                    || edit
                        .warnings
                        .as_ref()
                        .is_some_and(|warnings| !warnings.is_empty())
            })
            .cloned()
            .collect();
        (!visible.is_empty()).then_some(visible)
    }
}
