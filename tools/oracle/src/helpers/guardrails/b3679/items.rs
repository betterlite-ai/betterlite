use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::unified_dispatch::stream_items::{
    chunk_carries_text, chunk_has_finish_reason, chunk_has_tool_calls, is_trailing_metadata_chunk,
    tool_call_passthrough_chunk, trailing_metadata_chunks,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{reason_index, reason_pairs, to_json};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PredicateRequest {
    pub(super) items: Vec<GuardrailValue>,
}

pub(super) fn predicates(request: &PredicateRequest) -> Value {
    let items = &request.items;
    let trailing = trailing_metadata_chunks(items);
    let trailing_indexes: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| trailing.iter().any(|kept| std::ptr::eq(*kept, *item)))
        .map(|(index, _)| index)
        .collect();
    json!({
        "carries_text": items.iter().map(chunk_carries_text).collect::<Vec<bool>>(),
        "has_tool_calls": items.iter().map(chunk_has_tool_calls).collect::<Vec<bool>>(),
        "has_finish_reason": items.iter().map(chunk_has_finish_reason).collect::<Vec<bool>>(),
        "is_trailing_metadata": items.iter().map(is_trailing_metadata_chunk).collect::<Vec<bool>>(),
        "trailing": trailing_indexes,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassthroughRequest {
    pub(super) item: GuardrailValue,
    pub(super) reasons: Option<Vec<(i64, Option<String>)>>,
    #[serde(default)]
    pub(super) held: Vec<i64>,
}

pub(super) fn passthrough(request: &PassthroughRequest) -> HelperResult {
    let held: BTreeSet<i64> = request.held.iter().copied().collect();
    let mut reasons: Option<IndexMap<i64, Option<String>>> = request
        .reasons
        .as_ref()
        .map(|pairs| reason_index(pairs.as_slice()));
    let chunk = tool_call_passthrough_chunk(&request.item, reasons.as_mut(), &held);
    let output_reasons = reasons
        .as_ref()
        .map_or(Value::Null, |map| json!(reason_pairs(map)));
    Ok(json!({
        "chunk": to_json(&chunk)?,
        "reasons": output_reasons,
    }))
}
