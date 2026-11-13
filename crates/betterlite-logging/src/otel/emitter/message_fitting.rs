use std::collections::{BTreeMap, BTreeSet};

use crate::otel::mappers::base::AttributeMap;

const INPUT_FAMILY: &str = "llm.input_messages";
const OUTPUT_FAMILY: &str = "llm.output_messages";

// litellm.integrations.otel.mappers.openinference.fit_indexed_messages
#[must_use]
pub fn fit_indexed_messages(mut attrs: AttributeMap, budget: Option<usize>) -> AttributeMap {
    let Some(limit) = budget else {
        return attrs;
    };
    if attrs.len() <= limit {
        return attrs;
    }
    let groups = message_groups(&attrs);
    let order = shed_order(&groups);
    let excess = attrs.len().saturating_sub(limit);
    let shed_count = shed_count(&groups, &order, excess);
    let shed: BTreeSet<String> = order
        .iter()
        .take(shed_count)
        .filter_map(|group| groups.get(group))
        .flatten()
        .cloned()
        .collect();
    attrs.retain(|key, _| !shed.contains(key));
    attrs
}

fn message_groups(attrs: &AttributeMap) -> BTreeMap<(&'static str, i64), Vec<String>> {
    let mut groups: BTreeMap<(&'static str, i64), Vec<String>> = BTreeMap::new();
    for key in attrs.keys() {
        let Some((family, index)) = message_key(key) else {
            continue;
        };
        groups.entry((family, index)).or_default().push(key.clone());
    }
    groups
}

fn message_key(key: &str) -> Option<(&'static str, i64)> {
    let (family, rest) = match key.strip_prefix(INPUT_FAMILY) {
        Some(rest) => (INPUT_FAMILY, rest),
        None => (OUTPUT_FAMILY, key.strip_prefix(OUTPUT_FAMILY)?),
    };
    let index = rest.strip_prefix('.')?.split_once('.')?.0;
    Some((family, index.parse::<i64>().ok()?))
}

// middle prompt turns go first, then extra choices, the opener, the newest turn and the first choice
fn shed_order(groups: &BTreeMap<(&'static str, i64), Vec<String>>) -> Vec<(&'static str, i64)> {
    let inputs = indices(groups, INPUT_FAMILY);
    let outputs = indices(groups, OUTPUT_FAMILY);
    let mut order = Vec::new();
    for index in inputs.iter().skip(1).take(inputs.len().saturating_sub(2)) {
        order.push((INPUT_FAMILY, *index));
    }
    for index in outputs.iter().skip(1).rev() {
        order.push((OUTPUT_FAMILY, *index));
    }
    if let Some(index) = inputs.first() {
        order.push((INPUT_FAMILY, *index));
    }
    if let Some(index) = inputs.last()
        && inputs.first() != Some(index)
    {
        order.push((INPUT_FAMILY, *index));
    }
    if let Some(index) = outputs.first() {
        order.push((OUTPUT_FAMILY, *index));
    }
    order
}

fn indices(groups: &BTreeMap<(&'static str, i64), Vec<String>>, family: &'static str) -> Vec<i64> {
    let mut indices = Vec::new();
    for (name, index) in groups.keys() {
        if *name == family {
            indices.push(*index);
        }
    }
    indices
}

fn shed_count(
    groups: &BTreeMap<(&'static str, i64), Vec<String>>,
    order: &[(&'static str, i64)],
    excess: usize,
) -> usize {
    let mut running = 0_usize;
    for (position, group) in order.iter().enumerate() {
        running = running.saturating_add(groups.get(group).map_or(0, Vec::len));
        if running >= excess {
            return position.saturating_add(1);
        }
    }
    order.len()
}

#[path = "message_fitting/tests.rs"]
mod tests;
