use std::collections::BTreeMap;
use std::mem;

use crate::bedrock_records::{ContentItem, Extensions, Output, Response, TextContent, Usage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod tests;

pub const DEFAULT_CHUNK_BUDGET_CHARS: usize = 25_000;
const INTERVENED: &str = "GUARDRAIL_INTERVENED";
const TOO_LARGE: [&str; 6] = [
    "text unit",
    "maximum input size",
    "content size",
    "too long",
    "too large",
    "exceeds the maximum",
];

#[derive(Debug, Error)]
pub enum ChunkingError {
    #[error("there is no content item to split")]
    EmptyContent,
    #[error("the summed usage counter {counter} does not fit in 64 bits")]
    UsageOverflow { counter: String },
}

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
    let block = item.text.as_ref().and_then(|text| text.text.as_deref());
    block.unwrap_or("")
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

pub type Halves = (Vec<ContentItem>, Vec<ContentItem>);

pub fn split(content: &[ContentItem]) -> Result<Option<Halves>, ChunkingError> {
    if content.len() > 1 {
        let (left, right) = content.split_at(content.len().div_euclid(2));
        return Ok(Some((left.to_vec(), right.to_vec())));
    }
    let first = content.first().ok_or(ChunkingError::EmptyContent)?;
    let block = first.text.clone().unwrap_or_default();
    let text = block.text.unwrap_or_default();
    if text.chars().count() < 2 {
        return Ok(None);
    }
    let boundary = text.char_indices().nth(nearest_whitespace(&text));
    let (left, right) = text.split_at(boundary.map_or(text.len(), |(index, _)| index));
    let fragment = |part: &str| ContentItem {
        text: Some(TextContent {
            text: Some(part.to_owned()),
            qualifiers: block.qualifiers.clone().filter(|values| !values.is_empty()),
            extra: Extensions::new(),
        }),
        extra: Extensions::new(),
    };
    Ok(Some((vec![fragment(left)], vec![fragment(right)])))
}

#[must_use]
pub fn nearest_whitespace(text: &str) -> usize {
    let length = text.chars().count();
    let midpoint = length.div_euclid(2);
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
    let lowered = detail.map(str::to_lowercase).unwrap_or_default();
    TOO_LARGE.iter().any(|pattern| lowered.contains(pattern))
}

fn chunk_outputs(chunk: &ChunkResult) -> &[Output] {
    let response = &chunk.response;
    let outputs = response
        .outputs
        .as_deref()
        .filter(|values| !values.is_empty());
    outputs.or(response.output.as_deref()).unwrap_or_default()
}

fn merge_group(group: &[ChunkResult]) -> (Vec<Output>, bool) {
    let masked = group.iter().any(|chunk| !chunk_outputs(chunk).is_empty());
    if group.len() > 1 {
        let fragment_text = |chunk: &ChunkResult| {
            let masked_text = chunk_outputs(chunk)
                .first()
                .and_then(|output| output.text.clone());
            masked_text.unwrap_or_else(|| chunk.content.first().map_or("", source_text).to_owned())
        };
        let text = Some(group.iter().map(fragment_text).collect());
        let extra = Extensions::new();
        return (vec![Output { text, extra }], masked);
    }
    if masked {
        let outputs = group.iter().flat_map(chunk_outputs).cloned();
        return (outputs.collect(), true);
    }
    let sources = group.iter().flat_map(|chunk| &chunk.content);
    let placeholders = sources.map(|item| Output {
        text: Some(source_text(item).to_owned()),
        extra: Extensions::new(),
    });
    (placeholders.collect(), false)
}

fn sum_usage(chunks: &[ChunkResult]) -> Result<Option<Usage>, ChunkingError> {
    let usages = chunks
        .iter()
        .filter_map(|chunk| chunk.response.usage.as_ref());
    let mut totals: BTreeMap<String, Option<i64>> = BTreeMap::new();
    for (key, counter) in usages.flat_map(|usage| &usage.0) {
        let total = totals.entry(key.clone()).or_insert(Some(0));
        let sum = total
            .unwrap_or_default()
            .checked_add(counter.unwrap_or_default());
        let overflow = || ChunkingError::UsageOverflow {
            counter: key.clone(),
        };
        *total = Some(sum.ok_or_else(overflow)?);
    }
    Ok((!totals.is_empty()).then_some(Usage(totals)))
}

pub fn merge(chunks: &[ChunkResult]) -> Result<Response, ChunkingError> {
    let mut outputs = Vec::new();
    let mut any_masked = false;
    let mut remaining = chunks;
    while let Some(first) = remaining.first() {
        let size = first.fragment_group_size.clamp(1, remaining.len());
        let (group, rest) = remaining.split_at(size);
        let (unit_outputs, masked) = merge_group(group);
        outputs.extend(unit_outputs);
        any_masked |= masked;
        remaining = rest;
    }
    let mut merged = Response::default();
    let mut actions = Vec::new();
    let mut assessments = Vec::new();
    for chunk in chunks {
        merged.absorb(&chunk.response);
        actions.extend(chunk.response.action.clone());
        assessments.extend(chunk.response.assessments.clone().unwrap_or_default());
    }
    let intervened = actions.iter().any(|action| action == INTERVENED);
    let masked_outputs = (any_masked && !outputs.is_empty()).then_some(outputs);
    merged.absorb(&Response {
        usage: sum_usage(chunks)?,
        action: intervened
            .then(|| INTERVENED.to_owned())
            .or_else(|| actions.pop()),
        output: masked_outputs.clone(),
        outputs: masked_outputs,
        assessments: (!assessments.is_empty()).then_some(assessments),
        extra: Extensions::new(),
    });
    Ok(merged)
}
