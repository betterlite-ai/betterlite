use std::collections::BTreeMap;

use super::wire::{LassoContent, LassoMessage, ToolUseBlock};

pub(super) fn index_tool_use(masked: &[LassoMessage]) -> BTreeMap<String, ToolUseBlock> {
    let mut index = BTreeMap::new();
    for message in masked {
        if let LassoContent::ToolUse(block) = &message.content
            && let Some(id) = block.id.clone()
        {
            index.insert(id, block.clone());
        }
    }
    index
}

pub(super) fn index_tool_result(masked: &[LassoMessage]) -> BTreeMap<String, String> {
    let mut index = BTreeMap::new();
    for message in masked {
        if let LassoContent::ToolResult(block) = &message.content
            && let Some(id) = block.tool_use_id.clone()
        {
            index.insert(id, block.content.clone().unwrap_or_default());
        }
    }
    index
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
