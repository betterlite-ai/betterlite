use std::collections::BTreeMap;

use crate::bedrock_records::{
    BedrockContentItem, BedrockGuardrailOutput, BedrockGuardrailResponse,
};

#[cfg(test)]
mod tests;

pub const DEFAULT_CHUNK_BUDGET_CHARS: usize = 25_000;

#[derive(Debug, Clone, PartialEq)]
pub struct BedrockContentChunkResult {
    pub response: BedrockGuardrailResponse,
    pub content: Vec<BedrockContentItem>,
    pub fragment_group_size: usize,
}

pub fn content_uses_contextual_grounding(content: &[BedrockContentItem]) -> bool {
    content.iter().any(|item| {
        item.text
            .as_ref()
            .and_then(|text| text.qualifiers.as_ref())
            .is_some_and(|values| !values.is_empty())
    })
}

pub fn bin_pack_bedrock_content(
    content: &[BedrockContentItem],
    budget: usize,
) -> Vec<Vec<BedrockContentItem>> {
    if content.is_empty() {
        return vec![Vec::new()];
    }
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut used: usize = 0;
    for item in content {
        let length = item
            .text
            .as_ref()
            .and_then(|text| text.text.as_ref())
            .map_or(0, String::len);
        if !current.is_empty() && used.checked_add(length).is_some_and(|total| total > budget) {
            batches.push(current);
            current = Vec::new();
            used = 0;
        }
        used = used.saturating_add(length);
        current.push(item.clone());
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

pub fn split_bedrock_content(
    content: &[BedrockContentItem],
) -> Option<(Vec<BedrockContentItem>, Vec<BedrockContentItem>)> {
    if content.len() > 1 {
        let midpoint = content.len() / 2;
        return Some((content[..midpoint].to_vec(), content[midpoint..].to_vec()));
    }
    let item = content.first()?.clone();
    let text = item.text.as_ref()?.text.as_ref()?;
    if text.chars().count() < 2 {
        return None;
    }
    let split_at = nearest_whitespace_split_index(text);
    let (left, right) = text.split_at(split_at);
    let make = |part: &str| BedrockContentItem {
        text: Some(crate::bedrock_records::BedrockTextContent {
            text: Some(part.to_owned()),
            qualifiers: item
                .text
                .as_ref()
                .and_then(|value| value.qualifiers.clone()),
        }),
    };
    Some((vec![make(left)], vec![make(right)]))
}

pub fn nearest_whitespace_split_index(text: &str) -> usize {
    let midpoint = text
        .char_indices()
        .nth(text.chars().count() / 2)
        .map_or(text.len(), |(index, _)| index);
    let before = text
        .char_indices()
        .take_while(|(index, _)| *index < midpoint)
        .filter(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .last();
    let after = text
        .char_indices()
        .find(|(index, character)| *index >= midpoint && character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8());
    [before, after]
        .into_iter()
        .flatten()
        .filter(|index| *index > 0 && *index < text.len())
        .min_by_key(|index| index.abs_diff(midpoint))
        .unwrap_or(midpoint)
}

pub fn merge_bedrock_guardrail_responses(
    chunks: &[BedrockContentChunkResult],
) -> BedrockGuardrailResponse {
    let groups = group_fragment_units(chunks);
    let mut outputs = Vec::new();
    let mut masked = false;
    for group in groups {
        let (group_outputs, group_masked) = merge_group(group);
        outputs.extend(group_outputs);
        masked |= group_masked;
    }
    let mut merged = chunks
        .last()
        .map(|chunk| chunk.response.clone())
        .unwrap_or_default();
    let actions: Vec<&str> = chunks
        .iter()
        .filter_map(|chunk| chunk.response.action.as_deref())
        .collect();
    if actions
        .iter()
        .any(|action| *action == "GUARDRAIL_INTERVENED")
    {
        merged.action = Some("GUARDRAIL_INTERVENED".to_owned());
    } else if let Some(action) = actions.last() {
        merged.action = Some((*action).to_owned());
    }
    let assessments = chunks
        .iter()
        .flat_map(|chunk| chunk.response.assessments.clone().unwrap_or_default())
        .collect::<Vec<_>>();
    if !assessments.is_empty() {
        merged.assessments = Some(assessments);
    }
    if masked {
        merged.outputs = Some(outputs.clone());
        merged.output = Some(outputs);
    }
    if chunks.iter().any(|chunk| chunk.response.usage.is_some()) {
        merged.usage = Some(sum_usage(chunks));
    }
    merged
}

fn group_fragment_units(chunks: &[BedrockContentChunkResult]) -> Vec<&[BedrockContentChunkResult]> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < chunks.len() {
        let size = chunks[start]
            .fragment_group_size
            .max(1)
            .min(chunks.len() - start);
        groups.push(&chunks[start..start + size]);
        start += size;
    }
    groups
}

fn merge_group(group: &[BedrockContentChunkResult]) -> (Vec<BedrockGuardrailOutput>, bool) {
    if group.len() > 1 {
        let mut text = String::new();
        let mut masked = false;
        for chunk in group {
            let source = chunk
                .content
                .first()
                .and_then(|item| item.text.as_ref())
                .and_then(|text| text.text.as_deref())
                .unwrap_or("");
            let output = chunk
                .response
                .outputs
                .as_ref()
                .or(chunk.response.output.as_ref())
                .and_then(|values| values.first())
                .and_then(|value| value.text.as_deref())
                .unwrap_or(source);
            masked |= chunk
                .response
                .outputs
                .as_ref()
                .is_some_and(|values| !values.is_empty())
                || chunk
                    .response
                    .output
                    .as_ref()
                    .is_some_and(|values| !values.is_empty());
            text.push_str(output);
        }
        return (
            vec![BedrockGuardrailOutput {
                text: Some(text),
                extra: BTreeMap::new(),
            }],
            masked,
        );
    }
    let chunk = &group[0];
    let outputs = chunk
        .response
        .outputs
        .as_ref()
        .or(chunk.response.output.as_ref())
        .cloned()
        .unwrap_or_default();
    if outputs.len() == chunk.content.len() {
        return (outputs, !chunk.content.is_empty());
    }
    if outputs.is_empty() {
        return (
            chunk
                .content
                .iter()
                .map(|item| BedrockGuardrailOutput {
                    text: item.text.as_ref().and_then(|text| text.text.clone()),
                    extra: BTreeMap::new(),
                })
                .collect(),
            false,
        );
    }
    (outputs, true)
}

fn sum_usage(
    chunks: &[BedrockContentChunkResult],
) -> crate::bedrock_records::BedrockGuardrailUsage {
    let mut values: BTreeMap<String, i64> = BTreeMap::new();
    for chunk in chunks {
        if let Some(usage) = &chunk.response.usage {
            let fields = usage.extra.iter().filter_map(|(key, value)| match value {
                crate::bedrock_records::BedrockExtensionValue::Number(number) => {
                    Some((key.clone(), *number))
                }
                _ => None,
            });
            for (key, number) in fields {
                let entry = values.entry(key).or_insert(0);
                *entry = (*entry).saturating_add(number);
            }
        }
    }
    let mut result = crate::bedrock_records::BedrockGuardrailUsage::default();
    result.extra = values
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                crate::bedrock_records::BedrockExtensionValue::Number(value),
            )
        })
        .collect();
    result
}
