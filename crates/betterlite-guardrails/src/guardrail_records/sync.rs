use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::metadata::{RECORDS_KEY, metadata_key};
use super::record::GuardrailRecordContext;
use crate::guardrail_base::inputs::GuardrailLogTarget;

fn recorded_entries(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(bucket)) = request_data.get(metadata_key(request_data)) else {
        return Vec::new();
    };
    match bucket.get(RECORDS_KEY) {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(entries)) => entries.clone(),
        Some(single) => vec![single.clone()],
    }
}

// LiteLLM's _append_slg_to_litellm_params
fn append_missing(params: &mut BTreeMap<String, GuardrailValue>, entries: &[GuardrailValue]) {
    let metadata = params
        .entry("metadata".to_owned())
        .or_insert(GuardrailValue::Null);
    if *metadata == GuardrailValue::Null {
        *metadata = GuardrailValue::Object(BTreeMap::new());
    }
    let GuardrailValue::Object(fields) = metadata else {
        tracing::warn!("guardrail records were not copied: the logging metadata is not an object");
        return;
    };
    let recorded = fields
        .entry(RECORDS_KEY.to_owned())
        .or_insert_with(|| GuardrailValue::Array(Vec::new()));
    let GuardrailValue::Array(existing) = recorded else {
        tracing::warn!("guardrail records were not copied: the logging records are not a list");
        return;
    };
    for entry in entries {
        if !existing.contains(entry) {
            existing.push(entry.clone());
        }
    }
}

// LiteLLM's _sync_guardrail_info_to_logging_obj; reading never creates the bucket
pub fn sync_log_target(
    request_data: &BTreeMap<String, GuardrailValue>,
    target: &mut GuardrailLogTarget,
) {
    let entries = recorded_entries(request_data);
    if entries.is_empty() {
        return;
    }
    for params in [&mut target.litellm_params, &mut target.model_call_params]
        .into_iter()
        .flatten()
    {
        append_missing(params, &entries);
    }
}

pub(super) fn sync_context_target(context: &mut GuardrailRecordContext<'_>) {
    if let Some(target) = context.log_target.as_deref_mut() {
        sync_log_target(context.request_data, target);
    }
}
