use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_redact::structured::SensitiveDataMasker;

use crate::agent_credentials::REDACTED_BY_LITELM;

const MAX_DEPTH: usize = 10;

#[must_use]
// ast-grep-ignore: no-long-names, no-one-line-functions - the public entry fixes the signature; the walk carries a depth
pub fn restore_redacted_litellm_params(
    incoming: &BTreeMap<String, AgentJson>,
    existing: &BTreeMap<String, AgentJson>,
    masker: &SensitiveDataMasker,
) -> BTreeMap<String, AgentJson> {
    restored_fields(incoming, existing, masker, 0)
}

fn restored_fields(
    incoming: &BTreeMap<String, AgentJson>,
    existing: &BTreeMap<String, AgentJson>,
    masker: &SensitiveDataMasker,
    depth: usize,
) -> BTreeMap<String, AgentJson> {
    let mut restored = BTreeMap::new();
    for (key, value) in incoming {
        let Some(resolved) = restored_value(key, value, existing.get(key), masker, depth) else {
            continue;
        };
        restored.insert(key.clone(), resolved);
    }
    for (key, value) in existing {
        if !incoming.contains_key(key) && masker.is_sensitive_key(key, &[]) {
            restored.insert(key.clone(), value.clone());
        }
    }
    restored
}

fn restored_value(
    key: &str,
    value: &AgentJson,
    existing: Option<&AgentJson>,
    masker: &SensitiveDataMasker,
    depth: usize,
) -> Option<AgentJson> {
    if !masker.is_sensitive_key(key, &[]) {
        return Some(restored_nested(value, existing, masker, depth));
    }
    if is_marker(value) {
        return existing.cloned();
    }
    Some(value.clone())
}

fn restored_nested(
    incoming: &AgentJson,
    existing: Option<&AgentJson>,
    masker: &SensitiveDataMasker,
    depth: usize,
) -> AgentJson {
    if is_marker(incoming) && !existing.is_some_and(is_marker) {
        return existing.cloned().unwrap_or(AgentJson::Null);
    }
    if depth >= MAX_DEPTH {
        return incoming.clone();
    }
    match incoming {
        AgentJson::Object(_) => restored_object(incoming, existing, masker, depth),
        AgentJson::Array(items) => restored_array(items, existing, masker, depth),
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) | AgentJson::String(_) => {
            incoming.clone()
        }
    }
}

fn restored_object(
    incoming: &AgentJson,
    existing: Option<&AgentJson>,
    masker: &SensitiveDataMasker,
    depth: usize,
) -> AgentJson {
    let Some(fields) = incoming.as_object() else {
        return incoming.clone();
    };
    let incoming_fields: BTreeMap<String, AgentJson> = fields
        .iter()
        .map(|(key, nested)| (key.clone(), nested.clone()))
        .collect();
    AgentJson::Object(
        restored_fields(
            &incoming_fields,
            &existing_object_fields(existing),
            masker,
            depth.saturating_add(1),
        )
        .into_iter()
        .collect(),
    )
}

fn restored_array(
    items: &[AgentJson],
    existing: Option<&AgentJson>,
    masker: &SensitiveDataMasker,
    depth: usize,
) -> AgentJson {
    let existing_items = existing_array_items(existing);
    AgentJson::Array(
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                restored_nested(
                    item,
                    existing_items.get(index),
                    masker,
                    depth.saturating_add(1),
                )
            })
            .collect(),
    )
}

fn existing_object_fields(existing: Option<&AgentJson>) -> BTreeMap<String, AgentJson> {
    let Some(fields) = existing.and_then(AgentJson::as_object) else {
        return BTreeMap::new();
    };
    fields
        .iter()
        .map(|(key, nested)| (key.clone(), nested.clone()))
        .collect()
}

fn existing_array_items(existing: Option<&AgentJson>) -> &[AgentJson] {
    let Some(items) = existing.and_then(AgentJson::as_array) else {
        return &[];
    };
    items
}

fn is_marker(value: &AgentJson) -> bool {
    match value {
        AgentJson::String(text) => text == REDACTED_BY_LITELM,
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => false,
    }
}

mod tests;
