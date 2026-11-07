use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::value::{NodeId, SpendValue, to_json};

pub const MAX_PROJECTION_DEPTH: i32 = 20;
const MAX_DEPTH_EXCEEDED: &str = "<max_depth_exceeded>";
const CIRCULAR_REFERENCE: &str = "<circular_reference>";

#[must_use]
pub fn convert_to_json_serializable(value: &SpendValue) -> Json {
    let mut visited = BTreeSet::new();
    // A primitive at the top level passes through, since the default depth is never reached first
    match value {
        SpendValue::Null | SpendValue::Bool(_) | SpendValue::Number(_) | SpendValue::Text(_) => {
            to_json(value)
        }
        SpendValue::List(_)
        | SpendValue::Mapping(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => convert(value, &mut visited, MAX_PROJECTION_DEPTH),
    }
}

fn convert(value: &SpendValue, visited: &mut BTreeSet<NodeId>, depth: i32) -> Json {
    if depth <= 0_i32 {
        return Json::Text(MAX_DEPTH_EXCEEDED.to_owned());
    }
    match value {
        SpendValue::Null => Json::Null,
        SpendValue::Bool(flag) => Json::Bool(*flag),
        SpendValue::Number(number) => Json::Number(number.clone()),
        SpendValue::Text(text) => Json::Text(text.clone()),
        SpendValue::List(list) => {
            let Some(entered) = Entered::enter(visited, list.node) else {
                return Json::Text(CIRCULAR_REFERENCE.to_owned());
            };
            let items = list
                .items
                .iter()
                .map(|item| convert(item, entered.visited, depth.saturating_sub(1)))
                .collect();
            Json::List(items)
        }
        SpendValue::Mapping(mapping) => {
            let Some(entered) = Entered::enter(visited, mapping.node) else {
                return Json::Text(CIRCULAR_REFERENCE.to_owned());
            };
            let entries =
                convert_entries(&mapping.entries, entered.visited, depth.saturating_sub(1));
            Json::Object(entries)
        }
        SpendValue::Model(mapping) | SpendValue::Object(mapping) => {
            let Some(entered) = Entered::enter(visited, mapping.node) else {
                return Json::Text(CIRCULAR_REFERENCE.to_owned());
            };
            convert_dump(&mapping.entries, entered.visited, depth.saturating_sub(1))
        }
    }
}

// model_dump() and __dict__ are a second container, so a dump costs one more depth level
fn convert_dump(
    entries: &IndexMap<String, SpendValue>,
    visited: &mut BTreeSet<NodeId>,
    depth: i32,
) -> Json {
    if depth <= 0_i32 {
        return Json::Text(MAX_DEPTH_EXCEEDED.to_owned());
    }
    let mut dumped = IndexMap::with_capacity(entries.len());
    for (key, item) in entries {
        dumped.insert(key.clone(), convert(item, visited, depth.saturating_sub(1)));
    }
    Json::Object(dumped)
}

fn convert_entries(
    entries: &IndexMap<String, SpendValue>,
    visited: &mut BTreeSet<NodeId>,
    depth: i32,
) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, item)| (key.clone(), convert(item, visited, depth)))
        .collect()
}

// A container stays marked until its subtree is done, so a cycle back into it reads as circular
struct Entered<'visit> {
    visited: &'visit mut BTreeSet<NodeId>,
    node: NodeId,
}

impl<'visit> Entered<'visit> {
    fn enter(visited: &'visit mut BTreeSet<NodeId>, node: NodeId) -> Option<Self> {
        if !visited.insert(node) {
            return None;
        }
        Some(Self { visited, node })
    }
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        self.visited.remove(&self.node);
    }
}
