use std::slice;

use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::trace::SpanData;

use crate::langfuse_exporter::TRUNCATION_MARKER;
use crate::langfuse_observations::attributes::keys;

const INPUT_GROUP: [&str; 2] = [keys::OBSERVATION_INPUT, keys::TRACE_INPUT];
const OUTPUT_GROUP: [&str; 2] = [keys::OBSERVATION_OUTPUT, keys::TRACE_OUTPUT];
const METADATA_GROUP: [&str; 2] = [keys::OBSERVATION_METADATA, keys::TRACE_METADATA];
const TRUNCATION_GROUPS: [&[&str]; 3] = [&INPUT_GROUP, &OUTPUT_GROUP, &METADATA_GROUP];

// Langfuse reads input and output as one string but metadata only as flattened keys, so the marker gets one
#[must_use]
pub fn truncated(span: &SpanData) -> Option<SpanData> {
    let group = largest_group(&span.attributes)?;
    let mut attributes: Vec<KeyValue> = span
        .attributes
        .iter()
        .filter(|attribute| !in_group(attribute.key.as_str(), group))
        .cloned()
        .collect();
    let marked = group
        .iter()
        .copied()
        .filter(|prefix| prefix_present(&span.attributes, prefix))
        .map(|prefix| KeyValue::new(marker_key(prefix), Value::from(TRUNCATION_MARKER)));
    attributes.extend(marked);
    Some(SpanData {
        attributes,
        ..span.clone()
    })
}

// A tie keeps the first group, which is input, as Python's max() keeps the first of equal values
fn largest_group(attributes: &[KeyValue]) -> Option<&'static [&'static str]> {
    let mut largest: Option<(&'static [&'static str], usize)> = None;
    for group in TRUNCATION_GROUPS {
        let size = group_size(attributes, group);
        if largest.is_none_or(|(_, largest_size)| size > largest_size) {
            largest = Some((group, size));
        }
    }
    let (group, size) = largest?;
    (size > 0).then_some(group)
}

fn group_size(attributes: &[KeyValue], group: &[&str]) -> usize {
    attributes
        .iter()
        .filter(|attribute| {
            in_group(attribute.key.as_str(), group)
                && attribute.value.as_str().as_ref() != TRUNCATION_MARKER
        })
        .fold(0, |total, attribute| {
            total.saturating_add(attribute.value.as_str().chars().count())
        })
}

fn prefix_present(attributes: &[KeyValue], prefix: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| in_group(attribute.key.as_str(), slice::from_ref(&prefix)))
}

fn in_group(key: &str, group: &[&str]) -> bool {
    group.iter().any(|prefix| {
        key == *prefix
            || key
                .strip_prefix(*prefix)
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

fn marker_key(prefix: &str) -> String {
    if METADATA_GROUP.contains(&prefix) {
        format!("{prefix}.truncated")
    } else {
        prefix.to_owned()
    }
}
