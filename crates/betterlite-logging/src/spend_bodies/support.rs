use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, MAX_DEPTH_MARKER};
use crate::python_json;
use crate::spend_sanitize::value::{NodeId, SpendList, SpendMapping, SpendValue};

// Fresh mappings need identities that cannot collide with the ids a case or a request tree already uses
#[derive(Debug)]
pub struct NodeAllocator {
    next: u64,
}

impl Default for NodeAllocator {
    fn default() -> Self {
        Self { next: u64::MAX }
    }
}

impl NodeAllocator {
    pub fn allocate(&mut self) -> NodeId {
        let node = NodeId::from(self.next);
        self.next = self.next.saturating_sub(1);
        node
    }
}

pub(super) const fn as_mapping(value: &SpendValue) -> Option<&SpendMapping> {
    match value {
        SpendValue::Mapping(mapping) => Some(mapping),
        SpendValue::Null
        | SpendValue::Bool(_)
        | SpendValue::Number(_)
        | SpendValue::Text(_)
        | SpendValue::List(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => None,
    }
}

pub(super) const fn as_mapping_mut(value: &mut SpendValue) -> Option<&mut SpendMapping> {
    match value {
        SpendValue::Mapping(mapping) => Some(mapping),
        SpendValue::Null
        | SpendValue::Bool(_)
        | SpendValue::Number(_)
        | SpendValue::Text(_)
        | SpendValue::List(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => None,
    }
}

// LiteLLM asks `if value:` everywhere a policy flag or an overlay is read
pub(super) fn counts_as_present(value: &SpendValue) -> bool {
    match value {
        SpendValue::Null => false,
        SpendValue::Bool(flag) => *flag,
        SpendValue::Number(number) => number_is_nonzero(number),
        SpendValue::Text(text) => !text.is_empty(),
        SpendValue::List(list) => !list.items.is_empty(),
        SpendValue::Mapping(mapping) => !mapping.entries.is_empty(),
        // a foreign Python object has no length, so it counts as present
        SpendValue::Model(_) | SpendValue::Object(_) => true,
    }
}

fn number_is_nonzero(number: &Number) -> bool {
    number
        .to_string()
        .parse::<f64>()
        .map_or(true, |value| (value.to_bits() & 0x7fff_ffff_ffff_ffff) != 0)
}

// Python's str(), used where the source stringifies a trace id or a session id
pub(super) fn string_value(value: &SpendValue) -> String {
    match value {
        SpendValue::Null => "None".to_owned(),
        SpendValue::Bool(true) => "True".to_owned(),
        SpendValue::Bool(false) => "False".to_owned(),
        SpendValue::Number(number) => python_json::dumps(&Json::Number(number.clone())),
        SpendValue::Text(text) => text.clone(),
        SpendValue::List(_)
        | SpendValue::Mapping(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => representation(value),
    }
}

fn representation(value: &SpendValue) -> String {
    match value {
        SpendValue::Text(text) => quoted(text),
        SpendValue::List(list) => format!(
            "[{}]",
            list.items
                .iter()
                .map(representation)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SpendValue::Mapping(mapping) | SpendValue::Model(mapping) | SpendValue::Object(mapping) => {
            format!(
                "{{{}}}",
                mapping
                    .entries
                    .iter()
                    .map(|(key, item)| format!("{}: {}", quoted(key), representation(item)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        SpendValue::Null | SpendValue::Bool(_) | SpendValue::Number(_) => string_value(value),
    }
}

fn quoted(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len().saturating_add(2));
    out.push(quote);
    for character in text.chars() {
        if character == quote || character == '\\' {
            out.push('\\');
            out.push(character);
            continue;
        }
        match character {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(character),
        }
    }
    out.push(quote);
    out
}

// LiteLLM's safe_json_structure: strip NULs and replace anything past the depth bound
pub(super) fn safe_structure(value: &Json, depth: usize) -> Json {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Json::Text(MAX_DEPTH_MARKER.to_owned());
    }
    match value {
        Json::Text(text) => Json::Text(text.replace('\0', "")),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| safe_structure(item, depth.saturating_add(1)))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.replace('\0', ""),
                        safe_structure(item, depth.saturating_add(1)),
                    )
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
}

// Rebuild a JSON tree with allocator identities, so a snapshot merged into a request body cannot alias it
#[must_use]
pub fn spend_value(value: &Json, nodes: &mut NodeAllocator) -> SpendValue {
    match value {
        Json::Null => SpendValue::Null,
        Json::Bool(flag) => SpendValue::Bool(*flag),
        Json::Number(number) => SpendValue::Number(number.clone()),
        Json::Text(text) => SpendValue::Text(text.clone()),
        Json::List(items) => SpendValue::List(SpendList {
            node: nodes.allocate(),
            items: items.iter().map(|item| spend_value(item, nodes)).collect(),
        }),
        Json::Object(fields) => SpendValue::Mapping(SpendMapping {
            node: nodes.allocate(),
            entries: fields
                .iter()
                .map(|(key, item)| (key.clone(), spend_value(item, nodes)))
                .collect::<IndexMap<_, _>>(),
        }),
    }
}
