#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{MAX_DEPTH, resolve_mapping};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn wrapped(value: Json, levels: usize) -> IndexMap<String, Json> {
    let mut current = value;
    for _ in 0..levels {
        let mut wrapper = IndexMap::new();
        wrapper.insert("nested".to_owned(), current);
        current = Json::Object(wrapper);
    }
    let Json::Object(mapping) = current else {
        panic!("the fixture wraps a mapping");
    };
    mapping
}

fn innermost(value: &Json) -> &Json {
    let mut current = value;
    while let Json::Object(mapping) = current {
        let Some((_key, inner)) = mapping.first() else {
            panic!("the fixture holds a leaf");
        };
        current = inner;
    }
    current
}

#[test]
fn a_reference_in_a_mapping_at_the_depth_limit_is_resolved() {
    let mapping = wrapped(Json::Text("os.environ/PROMPT_URL".to_owned()), MAX_DEPTH);
    let resolved = Json::Object(resolve_mapping(
        mapping,
        1,
        &environment(&[("PROMPT_URL", "https://example.test")]),
    ));
    assert_eq!(
        innermost(&resolved),
        &Json::Text("https://example.test".to_owned())
    );
}

#[test]
fn a_reference_past_the_depth_limit_is_left_as_written() {
    let mapping = wrapped(
        Json::Text("os.environ/PROMPT_URL".to_owned()),
        MAX_DEPTH.saturating_add(1),
    );
    let resolved = Json::Object(resolve_mapping(
        mapping,
        1,
        &environment(&[("PROMPT_URL", "https://example.test")]),
    ));
    assert_eq!(
        innermost(&resolved),
        &Json::Text("os.environ/PROMPT_URL".to_owned())
    );
}
