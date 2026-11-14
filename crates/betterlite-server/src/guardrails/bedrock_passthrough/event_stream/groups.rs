use std::collections::BTreeMap;
use std::ops::{Div as _, Mul as _};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use num_traits::ToPrimitive as _;
use thiserror::Error;

use super::frames::Frame;

#[derive(Debug, Error)]
pub enum RedistributeError {
    #[error("a text length does not fit the share computation")]
    Length,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    Text,
    Tool,
    Reasoning,
    Citation(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub key: GroupKey,
    pub members: Vec<(usize, usize)>,
    pub fragments: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Groups {
    pub groups: Vec<Group>,
}

#[must_use]
pub fn group_texts(frames: &[Frame]) -> Groups {
    let mut groups: Vec<Group> = Vec::new();
    for (frame_index, frame) in frames.iter().enumerate() {
        for (cell_index, (key, text)) in frame.texts.iter().enumerate() {
            match groups.iter_mut().find(|group| group.key == *key) {
                Some(group) => {
                    group.members.push((frame_index, cell_index));
                    group.fragments.push(text.clone());
                }
                None => groups.push(Group {
                    key: key.clone(),
                    members: vec![(frame_index, cell_index)],
                    fragments: vec![text.clone()],
                }),
            }
        }
    }
    Groups { groups }
}

// The text cells a Converse contentBlockDelta carries, in the order the output handler gathers them.
#[must_use]
pub(super) fn delta_text_cells(delta: &GuardrailValue) -> Vec<(GroupKey, String)> {
    let GuardrailValue::Object(fields) = delta else {
        return Vec::new();
    };
    let mut cells = Vec::new();
    if let Some(GuardrailValue::Text(text)) = fields.get("text") {
        cells.push((GroupKey::Text, text.clone()));
    }
    if let Some(GuardrailValue::Object(tool_use)) = fields.get("toolUse")
        && let Some(GuardrailValue::Text(text)) = tool_use.get("input")
    {
        cells.push((GroupKey::Tool, text.clone()));
    }
    if let Some(GuardrailValue::Object(reasoning)) = fields.get("reasoningContent")
        && let Some(GuardrailValue::Text(text)) = reasoning.get("text")
    {
        cells.push((GroupKey::Reasoning, text.clone()));
    }
    let Some(GuardrailValue::Object(citations)) = fields.get("citationsContent") else {
        return cells;
    };
    let Some(GuardrailValue::Array(cited_blocks)) = citations.get("content") else {
        return cells;
    };
    for (index, entry) in cited_blocks.iter().enumerate() {
        if let GuardrailValue::Object(cited) = entry
            && let Some(GuardrailValue::Text(text)) = cited.get("text")
        {
            cells.push((GroupKey::Citation(index), text.clone()));
        }
    }
    cells
}

pub fn redistribute(
    groups: &Groups,
    returned: &[String],
) -> Result<BTreeMap<(usize, usize), String>, RedistributeError> {
    let mut new_texts = BTreeMap::new();
    let active = groups
        .groups
        .iter()
        .filter(|group| !group.fragments.concat().is_empty());
    for (group, replacement) in active.zip(returned) {
        split_group(group, replacement, &mut new_texts)?;
    }
    Ok(new_texts)
}

fn split_group(
    group: &Group,
    replacement: &str,
    new_texts: &mut BTreeMap<(usize, usize), String>,
) -> Result<(), RedistributeError> {
    let originals: Vec<usize> = group
        .fragments
        .iter()
        .map(|fragment| fragment.chars().count())
        .collect();
    let total = originals
        .iter()
        .copied()
        .fold(0_usize, usize::saturating_add)
        .max(1);
    let characters: Vec<char> = replacement.chars().collect();
    let new_len = characters.len();
    let last = group.members.len().saturating_sub(1);
    let mut position = 0_usize;
    for (index, (member, original)) in group.members.iter().zip(&originals).enumerate() {
        let fragment: String = if index == last {
            characters.iter().skip(position).copied().collect()
        } else {
            let end = position
                .saturating_add(rounded_share(new_len, *original, total)?)
                .min(new_len);
            let text = characters
                .iter()
                .skip(position)
                .take(end.saturating_sub(position))
                .copied()
                .collect();
            position = end;
            text
        };
        new_texts.insert(*member, fragment);
    }
    Ok(())
}

pub(super) fn write_delta_cells(
    delta: &mut GuardrailValue,
    frame_index: usize,
    new_texts: &BTreeMap<(usize, usize), String>,
) {
    let GuardrailValue::Object(fields) = delta else {
        return;
    };
    let mut cell_index = 0_usize;
    if let Some(GuardrailValue::Text(text)) = fields.get_mut("text") {
        apply_replacement(text, (frame_index, cell_index), new_texts);
        cell_index = cell_index.saturating_add(1);
    }
    if let Some(GuardrailValue::Object(tool_use)) = fields.get_mut("toolUse")
        && let Some(GuardrailValue::Text(text)) = tool_use.get_mut("input")
    {
        apply_replacement(text, (frame_index, cell_index), new_texts);
        cell_index = cell_index.saturating_add(1);
    }
    if let Some(GuardrailValue::Object(reasoning)) = fields.get_mut("reasoningContent")
        && let Some(GuardrailValue::Text(text)) = reasoning.get_mut("text")
    {
        apply_replacement(text, (frame_index, cell_index), new_texts);
        cell_index = cell_index.saturating_add(1);
    }
    let Some(GuardrailValue::Object(citations)) = fields.get_mut("citationsContent") else {
        return;
    };
    let Some(GuardrailValue::Array(cited_blocks)) = citations.get_mut("content") else {
        return;
    };
    for entry in cited_blocks.iter_mut() {
        if let GuardrailValue::Object(cited) = entry
            && let Some(GuardrailValue::Text(text)) = cited.get_mut("text")
        {
            apply_replacement(text, (frame_index, cell_index), new_texts);
            cell_index = cell_index.saturating_add(1);
        }
    }
}

fn apply_replacement(
    text: &mut String,
    key: (usize, usize),
    new_texts: &BTreeMap<(usize, usize), String>,
) {
    if let Some(replacement) = new_texts.get(&key) {
        text.clone_from(replacement);
    }
}

// Python's round() breaks a tie toward the even integer, and the quotient is a float there.
fn rounded_share(
    new_len: usize,
    original: usize,
    total: usize,
) -> Result<usize, RedistributeError> {
    let share = new_len
        .to_f64()
        .ok_or(RedistributeError::Length)?
        .mul(original.to_f64().ok_or(RedistributeError::Length)?)
        .div(total.to_f64().ok_or(RedistributeError::Length)?)
        .round_ties_even();
    share.to_usize().ok_or(RedistributeError::Length)
}
