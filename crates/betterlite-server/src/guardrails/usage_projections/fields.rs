use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use super::records::GuardrailRecord;

// what LiteLLM shows when a record leaves its provider or kind out
pub const UNKNOWN_PROVIDER: &str = "Unknown";
pub const DEFAULT_GUARDRAIL_TYPE: &str = "Guardrail";

static EMPTY_PARAMS: OpaqueParams = OpaqueParams(BTreeMap::new());

/// `mapping[key]` as text, or `default` when the key is absent or holds something else.
#[must_use]
pub fn field_str(mapping: &OpaqueParams, key: &str, default: &str) -> String {
    match mapping.0.get(key) {
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(_) | None => default.to_owned(),
    }
}

/// The guardrail the record runs, from `litellm_params.guardrail`.
#[must_use]
pub fn guardrail_provider(record: &GuardrailRecord) -> String {
    record
        .litellm_params
        .guardrail
        .as_deref()
        .unwrap_or(UNKNOWN_PROVIDER)
        .to_owned()
}

/// The record's kind, from `guardrail_info.type`; an absent info reads as empty.
#[must_use]
pub fn guardrail_type(record: &GuardrailRecord) -> String {
    field_str(
        record.guardrail_info.as_ref().unwrap_or(&EMPTY_PARAMS),
        "type",
        DEFAULT_GUARDRAIL_TYPE,
    )
}

/// The record's id and display name; the name falls back to the id, then to empty.
#[must_use]
pub fn guardrail_attrs(record: &GuardrailRecord) -> (String, String) {
    let id = record.guardrail_id.clone().unwrap_or_default();
    let name = match record.guardrail_name.as_deref() {
        Some(name) if !name.is_empty() => name.to_owned(),
        Some(_) | None => id.clone(),
    };
    (id, name)
}

mod tests;
