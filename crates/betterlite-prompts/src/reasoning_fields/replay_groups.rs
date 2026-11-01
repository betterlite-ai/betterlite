use super::encrypted_signature::{encrypted_content_of_block, is_encrypted_reasoning_block};
use super::thinking_text::readable_thinking_text;
use crate::message_content::ContentPart;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBlockKind {
    SummaryText,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReasoningSummaryTextBlock {
    #[serde(rename = "type")]
    pub kind: SummaryBlockKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningItemKind {
    Reasoning,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReasoningItem {
    #[serde(rename = "type")]
    pub kind: ReasoningItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    pub summary: Vec<ReasoningSummaryTextBlock>,
}

// No item carries an id: the Responses API 404s on an id it did not mint itself but accepts an item with none.
#[must_use]
pub fn reasoning_items_from_blocks(thinking_blocks: &[ContentPart]) -> Vec<ReasoningItem> {
    group_by_replay_key(thinking_blocks)
        .iter()
        .filter_map(|group| reasoning_item_from_group(group))
        .collect()
}

fn replay_group_key(index: usize, block: &ContentPart) -> String {
    if is_encrypted_reasoning_block(block) {
        format!("encrypted:{index}")
    } else {
        "summary".to_owned()
    }
}

// Mirrors itertools.groupby: only *consecutive* equal keys merge into one group.
fn group_by_replay_key(blocks: &[ContentPart]) -> Vec<Vec<ContentPart>> {
    let mut groups: Vec<(String, Vec<ContentPart>)> = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let key = replay_group_key(index, block);
        match groups.last_mut() {
            Some((last_key, group)) if *last_key == key => group.push(block.clone()),
            _ => groups.push((key, vec![block.clone()])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

fn reasoning_item_from_group(group: &[ContentPart]) -> Option<ReasoningItem> {
    let summary: Vec<ReasoningSummaryTextBlock> = group
        .iter()
        .map(readable_thinking_text)
        .filter(|text| !text.is_empty())
        .map(|text| ReasoningSummaryTextBlock {
            kind: SummaryBlockKind::SummaryText,
            text,
        })
        .collect();
    let encrypted_content = group.first().and_then(encrypted_content_of_block);
    if encrypted_content.is_some() {
        return Some(ReasoningItem {
            kind: ReasoningItemKind::Reasoning,
            encrypted_content,
            summary,
        });
    }
    if summary.is_empty() {
        return None;
    }
    Some(ReasoningItem {
        kind: ReasoningItemKind::Reasoning,
        encrypted_content: None,
        summary,
    })
}

mod tests;
