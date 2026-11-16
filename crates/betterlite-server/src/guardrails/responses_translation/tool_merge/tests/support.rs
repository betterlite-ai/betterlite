use std::collections::BTreeMap;

use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_models::decisions::Json;

use crate::guardrails::responses_translation::tool_merge::as_tool::namespace_members;
use crate::guardrails::responses_translation::tool_merge::group::FlattenedGroup;
use crate::guardrails::responses_translation::tool_merge::indexed_key::{IndexedKey, indexed_keys};
use crate::guardrails::responses_translation::tool_merge::rebuild_namespace::rebuilt_namespace;

pub(crate) const NAMESPACE: &str = r#"{"type": "namespace", "name": "crm", "description": "CRM tools", "tools": [
    {"type": "function", "name": "lookup", "description": "Find a contact"},
    {"type": "custom", "name": "note", "description": "Write a note",
     "format": {"type": "grammar", "syntax": "lark", "definition": "start: WORD"}},
    {"type": "web_search"}]}"#;

pub(crate) const FLATTENED: &str = r#"[
    {"type": "function", "function": {"name": "crm__lookup", "description": "CRM tools\n\nFind a contact", "parameters": {"type": "object"}}},
    {"type": "function", "function": {"name": "crm__note", "description": "CRM tools\n\nWrite a note\n\nFormat:\n```lark\nstart: WORD\n```", "parameters": {"type": "object"}}}]"#;

pub(crate) fn fields(source: &str) -> Fields {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn field_list(source: &str) -> Vec<Fields> {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn json_list(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn keys_for(tools: &[Fields]) -> Vec<IndexedKey> {
    let references: Vec<&Fields> = tools.iter().collect();
    indexed_keys(&references).unwrap()
}

// Rebuilds the original namespace from what the guardrail returned for its flattened tools.
pub(crate) fn rebuild(
    original_source: &str,
    flattened_source: &str,
    guardrailed_source: &str,
) -> Vec<Fields> {
    let original = fields(original_source);
    let flattened = field_list(flattened_source);
    let guardrailed = field_list(guardrailed_source);
    let flattened_keys = keys_for(&flattened);
    let guardrailed_by_key: BTreeMap<IndexedKey, &Fields> = keys_for(&guardrailed)
        .into_iter()
        .zip(guardrailed.iter())
        .collect();
    let group = FlattenedGroup {
        tools: &flattened,
        keys: &flattened_keys,
        guardrailed: &guardrailed_by_key,
    };
    let members = namespace_members(&original);
    rebuilt_namespace(&original, &members, &group).unwrap()
}
