use std::collections::BTreeMap;

use indexmap::IndexMap;

use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_analysis::wire::AnalyzeResponseItem;

// LiteLLM numbers entities left to right and replaces spans from the end so earlier offsets stay valid.
pub(super) struct NumberedText {
    pub(super) text: String,
    pub(super) tokens: IndexMap<String, String>,
}

pub(super) fn number_spans(
    text: &str,
    analyze_results: &[AnalyzeResponseItem],
    masked_entity_count: &mut IndexMap<String, usize>,
) -> Result<NumberedText, PresidioActionsError> {
    let original_chars: Vec<char> = text.chars().collect();
    let spans = collect_spans(analyze_results, original_chars.len())?;
    let sequence = sequence_numbers(&spans);
    let mut tokens = IndexMap::new();
    let mut replaced_text = original_chars.clone();
    for (start, end, entity_type) in spans.iter().rev() {
        let Some(sequence_number) = sequence.get(&(*start, *end)) else {
            return Err(PresidioActionsError::SpanOutOfBounds {
                start: *start,
                end: *end,
            });
        };
        let token = format!("<{entity_type}_{sequence_number}>");
        let Some(original_slice) = original_chars.get(*start..*end) else {
            return Err(PresidioActionsError::SpanOutOfBounds {
                start: *start,
                end: *end,
            });
        };
        // LiteLLM stores text[start:end] from the input, while replacements land on the mutated copy.
        let original: String = original_slice.iter().collect();
        replaced_text = replace_span(&replaced_text, *start, *end, &token);
        tokens.insert(token, original);
        let count = masked_entity_count
            .entry((*entity_type).to_owned())
            .or_insert(0);
        *count = count.saturating_add(1);
    }
    Ok(NumberedText {
        text: replaced_text.into_iter().collect(),
        tokens,
    })
}

fn collect_spans(
    analyze_results: &[AnalyzeResponseItem],
    char_count: usize,
) -> Result<Vec<(usize, usize, &str)>, PresidioActionsError> {
    let mut spans = Vec::with_capacity(analyze_results.len());
    for item in analyze_results {
        let start = item
            .start
            .ok_or(PresidioActionsError::MissingSpan { field: "start" })?;
        let end = item
            .end
            .ok_or(PresidioActionsError::MissingSpan { field: "end" })?;
        if start > end || end > char_count {
            return Err(PresidioActionsError::SpanOutOfBounds { start, end });
        }
        let entity_type = item
            .entity_type
            .as_deref()
            .ok_or(PresidioActionsError::MissingEntityType)?;
        spans.push((start, end, entity_type));
    }
    spans.sort_by_key(|(start, _, _)| *start);
    Ok(spans)
}

fn sequence_numbers(spans: &[(usize, usize, &str)]) -> BTreeMap<(usize, usize), usize> {
    let mut sequence = BTreeMap::new();
    for (index, (start, end, _)) in spans.iter().enumerate() {
        // A repeated span takes the last sequence number, because LiteLLM fills a dict keyed by span.
        sequence.insert((*start, *end), index.saturating_add(1));
    }
    sequence
}

// Python slices clamp, so a span that no longer fits the shrunken copy keeps the prefix and drops the tail.
fn replace_span(chars: &[char], start: usize, end: usize, token: &str) -> Vec<char> {
    let mut replaced: Vec<char> = chars.iter().take(start).copied().collect();
    replaced.extend(token.chars());
    replaced.extend(chars.iter().skip(end).copied());
    replaced
}

#[must_use]
pub fn unmask_pii_text(text: &str, pii_tokens: &IndexMap<String, String>) -> String {
    let mut current = text.to_owned();
    for (token, original) in pii_tokens {
        if token.is_empty() {
            continue;
        }
        if current.contains(token.as_str()) {
            current = current.replace(token.as_str(), original.as_str());
            continue;
        }
        let chars: Vec<char> = current.chars().collect();
        if let Some(index) = truncated_suffix(&chars, token) {
            let mut combined: String = chars.iter().take(index).collect();
            combined.push_str(original);
            current = combined;
        }
    }
    current
}

fn truncated_suffix(text: &[char], token: &str) -> Option<usize> {
    let token_chars: Vec<char> = token.chars().collect();
    let min_overlap = (token_chars.len() >> 1_usize).min(20_usize);
    let start = text.len().saturating_sub(token_chars.len());
    for index in start..text.len() {
        let Some(suffix) = text.get(index..) else {
            continue;
        };
        if suffix.len() >= min_overlap && token_chars.starts_with(suffix) {
            return Some(index);
        }
    }
    None
}
