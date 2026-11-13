use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::unified_dispatch::stream_items::{
    chunk_carries_text, chunk_has_finish_reason, chunk_has_tool_calls, is_trailing_metadata_chunk,
    tool_call_passthrough_chunk,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{reason_index, reason_pairs, to_json};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PredicateRequest {
    pub(super) item: GuardrailValue,
}

pub(super) fn predicates(request: &PredicateRequest) -> Value {
    json!({
        "carries_text": chunk_carries_text(&request.item),
        "has_tool_calls": chunk_has_tool_calls(&request.item),
        "has_finish_reason": chunk_has_finish_reason(&request.item),
        "is_trailing_metadata": is_trailing_metadata_chunk(&request.item),
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
