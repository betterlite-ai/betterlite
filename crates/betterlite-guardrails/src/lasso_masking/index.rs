use std::collections::BTreeMap;

use super::wire::{LassoContent, LassoMessage, ToolUseBlock};

pub(super) struct ToolIndex {
    pub(super) tool_use: BTreeMap<String, ToolUseBlock>,
    pub(super) tool_result: BTreeMap<String, String>,
}

// One pass over the masked messages fills both maps, since a message's content is exactly one of ToolUse, ToolResult or Text.
pub(super) fn index_tool_blocks(masked: &[LassoMessage]) -> ToolIndex {
    let mut tool_use = BTreeMap::new();
    let mut tool_result = BTreeMap::new();
    for message in masked {
        match &message.content {
            LassoContent::ToolUse(block) => {
                if let Some(id) = block.id.clone() {
                    tool_use.insert(id, block.clone());
                }
            }
            LassoContent::ToolResult(block) => {
                if let Some(id) = block.tool_use_id.clone() {
                    tool_result.insert(id, block.content.clone().unwrap_or_default());
                }
            }
            LassoContent::Text(_) => {}
        }
    }
    ToolIndex {
        tool_use,
        tool_result,
    }
}

pub(super) fn collect_texts(masked: &[LassoMessage]) -> Vec<String> {
    let mut texts = Vec::new();
    for message in masked {
        if let LassoContent::Text(text) = &message.content {
            texts.push(text.clone());
        }
    }
    texts
}

// Walks texts positionally, but only while the caller's count still matches Lasso's: a mismatch means Lasso dropped or reordered messages, so mapping by position would write masked content onto the wrong original message.
pub(super) struct TextCursor<'texts> {
    pub(super) texts: &'texts [String],
    pub(super) index: usize,
    pub(super) enabled: bool,
}

impl TextCursor<'_> {
    pub(super) fn next_text(&mut self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let text = self.texts.get(self.index)?.clone();
        self.index = self.index.saturating_add(1);
        Some(text)
    }
}
