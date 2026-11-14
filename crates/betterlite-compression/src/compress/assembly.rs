use indexmap::IndexMap;
use serde_json::Value;
use std::collections::BTreeSet;

use crate::extract_key::extract_key;
use crate::message_normalization::content_to_text;
use crate::message_stubbing::stub_message;
use crate::retrieval_tool::build_retrieval_tools;

use super::CompressError;
use super::CompressedResult;
use super::PreparedCompression;
use super::budget::BudgetSelection;
use super::counting::TokenCounter;

pub(super) fn get_dropped_tool_span_indices(
    kept_indices: &BTreeSet<usize>,
    tool_exchange_spans: &[Vec<usize>],
) -> BTreeSet<usize> {
    let mut dropped: BTreeSet<usize> = BTreeSet::new();
    for span in tool_exchange_spans {
        if !span.iter().any(|index| kept_indices.contains(index)) {
            dropped.extend(span.iter().copied());
        }
    }
    dropped
}

pub(super) async fn assemble_result<Tokenizer>(
    prepared: &PreparedCompression,
    selection: &BudgetSelection,
    tool_exchange_spans: &[Vec<usize>],
    model: &str,
    counter: &Tokenizer,
) -> Result<CompressedResult, CompressError>
where
    Tokenizer: TokenCounter,
{
    let dropped_tool_span_indices =
        get_dropped_tool_span_indices(&selection.kept_indices, tool_exchange_spans);
    let (messages, cache) = assemble_messages(prepared, selection, &dropped_tool_span_indices);
    let available_keys: Vec<String> = cache.keys().cloned().collect();
    let tools = build_retrieval_tools(&available_keys, prepared.call_type)?;
    let compressed_tokens = counter
        .count_messages(model, &messages)
        .await
        .map_err(CompressError::Counting)?;
    let compression_ratio = compression_ratio(prepared.original_tokens, compressed_tokens);
    Ok(CompressedResult {
        messages,
        original_tokens: prepared.original_tokens,
        compressed_tokens,
        compression_ratio,
        cache,
        tools,
        compression_skipped_reason: None,
    })
}

fn compression_ratio(original_tokens: usize, compressed_tokens: usize) -> f64 {
    if original_tokens == 0 {
        return 0.0;
    }
    let compressed =
        u32::try_from(compressed_tokens).map_or_else(|_| f64::from(u32::MAX), f64::from);
    let original = u32::try_from(original_tokens).map_or_else(|_| f64::from(u32::MAX), f64::from);
    // Python's round() breaks a tie to even, so 0.03125 keeps 0.0312
    ((1.0 - compressed / original) * 10_000.0).round_ties_even() / 10_000.0
}

fn assemble_messages(
    prepared: &PreparedCompression,
    selection: &BudgetSelection,
    dropped_tool_span_indices: &BTreeSet<usize>,
) -> (Vec<Value>, IndexMap<String, String>) {
    let mut messages: Vec<Value> = Vec::with_capacity(prepared.original_messages.len());
    let mut cache: IndexMap<String, String> = IndexMap::new();
    let mut used_keys: Vec<String> = Vec::new();
    for (index, (original, normalized)) in prepared
        .original_messages
        .iter()
        .zip(&prepared.normalized_messages)
        .enumerate()
    {
        if dropped_tool_span_indices.contains(&index) {
            continue;
        }
        if selection.kept_indices.contains(&index) {
            let kept = selection
                .truncated_overrides
                .get(&index)
                .map_or_else(|| original.clone(), Clone::clone);
            messages.push(kept);
            continue;
        }
        let key = extract_key(normalized, index, &mut used_keys);
        let content = original
            .get("content")
            .map(content_to_text)
            .unwrap_or_default();
        cache.insert(key.clone(), content);
        messages.push(stub_message(original, &key));
    }
    (messages, cache)
}
