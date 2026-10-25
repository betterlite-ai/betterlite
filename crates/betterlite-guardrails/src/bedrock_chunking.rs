use std::collections::BTreeMap;
use std::mem;

use crate::bedrock_records::{ContentItem, Output, Response, TextContent, Usage};
use serde::{Deserialize, Serialize};

mod tests;

pub const DEFAULT_CHUNK_BUDGET_CHARS: usize = 25_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResult {
    pub response: Response,
    pub content: Vec<ContentItem>,
    pub fragment_group_size: usize,
}

#[must_use]
pub fn uses_grounding(content: &[ContentItem]) -> bool {
    let mut qualifiers = content
        .iter()
        .filter_map(|item| item.text.as_ref()?.qualifiers.as_ref());
    qualifiers.any(|values| !values.is_empty())
}

fn source_text(item: &ContentItem) -> &str {
    let Some(block) = &item.text else {
        return "";
    };
    block.text.as_deref().unwrap_or("")
}

#[must_use]
pub fn bin_pack(content: &[ContentItem], budget: usize) -> Vec<Vec<ContentItem>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut used: usize = 0;
    for item in content {
        let length = source_text(item).chars().count();
        if !current.is_empty() && used.saturating_add(length) > budget {
            batches.push(mem::take(&mut current));
            used = 0;
        }
        used = used.saturating_add(length);
        current.push(item.clone());
    }
    if !current.is_empty() || batches.is_empty() {
        batches.push(current);
    }
    batches
}

#[must_use]
pub fn split(content: &[ContentItem]) -> Option<(Vec<ContentItem>, Vec<ContentItem>)> {
    if content.len() > 1 {
        let (left, right) = content.split_at_checked(content.len().checked_div(2)?)?;
        return Some((left.to_vec(), right.to_vec()));
    }
    let block = content.first()?.text.as_ref()?;
    let text = block.text.as_ref()?;
    if text.chars().count() < 2 {
        return None;
    }
    let boundary = text.char_indices().nth(nearest_whitespace(text))?.0;
    let (left, right) = text.split_at_checked(boundary)?;
    let fragment = |part: &str| ContentItem {
        text: Some(TextContent {
            text: Some(part.to_owned()),
            qualifiers: block.qualifiers.clone().filter(|values| !values.is_empty()),
            extra: BTreeMap::new(),
        }),
        extra: BTreeMap::new(),
    };
    Some((vec![fragment(left)], vec![fragment(right)]))
}

#[must_use]
pub fn nearest_whitespace(text: &str) -> usize {
    let length = text.chars().count();
    let midpoint = length.checked_shr(1).unwrap_or_default();
    let whitespace =
        |character: char| character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character);
    let before = text
        .chars()
        .enumerate()
        .take(midpoint)
        .filter(|(_, character)| whitespace(*character))
        .map(|(index, _)| index)
        .last();
    let after = text
        .chars()
        .enumerate()
        .skip(midpoint)
        .find(|(_, character)| whitespace(*character))
        .map(|(index, _)| index);
    [before, after]
        .into_iter()
        .flatten()
        .map(|index| index.saturating_add(1))
        .filter(|index| *index < length)
        .min_by_key(|index| index.abs_diff(midpoint))
        .unwrap_or(midpoint)
}

#[must_use]
pub fn is_input_too_large(detail: Option<&str>) -> bool {
    let Some(message) = detail else {
        return false;
    };
    let lowered = message.to_lowercase();
    [
        "text unit",
        "maximum input size",
        "content size",
        "too long",
        "too large",
        "exceeds the maximum",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern))
}

fn chunk_outputs(chunk: &ChunkResult) -> &[Output] {
    let response = &chunk.response;
    response
        .outputs
        .as_deref()
        .filter(|values| !values.is_empty())
        .or(response.output.as_deref())
        .unwrap_or_default()
}

fn merge_group(group: &[ChunkResult]) -> (Vec<Output>, bool) {
    let Some(first) = group.first() else {
        return (Vec::new(), false);
    };
    let masked = group.iter().any(|chunk| !chunk_outputs(chunk).is_empty());
    if group.len() > 1 {
        let text = group
            .iter()
            .map(|chunk| {
                chunk_outputs(chunk)
                    .first()
                    .and_then(|output| output.text.as_deref())
                    .unwrap_or_else(|| chunk.content.first().map_or("", source_text))
            })
            .collect();
        return (
            vec![Output {
                text: Some(text),
                extra: BTreeMap::new(),
            }],
            masked,
        );
    }
    let outputs = chunk_outputs(first);
    if !outputs.is_empty() {
        return (outputs.to_vec(), true);
    }
    (
        first
            .content
            .iter()
            .map(|item| Output {
                text: Some(source_text(item).to_owned()),
                extra: BTreeMap::new(),
            })
            .collect(),
        false,
    )
}

#[must_use]
pub fn merge(chunks: &[ChunkResult]) -> Response {
    let mut merged = Response::default();
    let mut outputs = Vec::new();
    let mut any_masked = false;
    let mut remaining = chunks;
    while let Some(first) = remaining.first() {
        let size = first.fragment_group_size.max(1).min(remaining.len());
        let Some((group, rest)) = remaining.split_at_checked(size) else {
            break;
        };
        let (unit_outputs, masked) = merge_group(group);
        outputs.extend(unit_outputs);
        any_masked |= masked;
        remaining = rest;
    }
    for chunk in chunks {
        merge_fields(&mut merged, &chunk.response);
    }
    if any_masked && !outputs.is_empty() {
        merged.output = Some(outputs.clone());
        merged.outputs = Some(outputs);
    }
    merged
}

fn merge_fields(merged: &mut Response, response: &Response) {
    merged.extra.extend(response.extra.clone());
    if merged.action.as_deref() != Some("GUARDRAIL_INTERVENED") && response.action.is_some() {
        merged.action.clone_from(&response.action);
    }
    if let Some(outputs) = &response.outputs {
        merged.outputs = Some(outputs.clone());
    }
    if let Some(output) = &response.output {
        merged.output = Some(output.clone());
    }
    if let Some(assessments) = &response.assessments {
        merged
            .assessments
            .get_or_insert_default()
            .extend(assessments.clone());
    }
    if let Some(usage) = &response.usage {
        let totals = merged.usage.get_or_insert_with(Usage::default);
        for (key, counter) in &usage.0 {
            let entry = totals.0.entry(key.clone()).or_insert(Some(0));
            *entry = Some(
                entry
                    .unwrap_or_default()
                    .saturating_add(counter.unwrap_or_default()),
            );
        }
    }
}
