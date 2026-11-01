#![cfg(test)]

use std::collections::BTreeMap;

use serde::Deserialize;

use super::MappingOrScalar;
use crate::tiered_pricing::Scalar;

// A plain derive here would also accept a one-element array positionally; try_from is the pattern every real mapping type in this module relies on to reject that instead.
#[derive(Debug, Deserialize)]
#[serde(try_from = "BTreeMap<String, Scalar>")]
struct Stats {
    tokens_saved: Option<i64>,
}

impl From<BTreeMap<String, Scalar>> for Stats {
    fn from(mut map: BTreeMap<String, Scalar>) -> Self {
        let tokens_saved = match map.remove("tokens_saved") {
            Some(Scalar::Number(number)) => number.as_i64(),
            _ => None,
        };
        Self { tokens_saved }
    }
}

#[test]
fn as_mapping_reads_a_deserialized_object() {
    let parsed: MappingOrScalar<Stats> =
        serde_json::from_str(r#"{"tokens_saved": 5}"#).expect("an object deserializes");
    assert_eq!(
        parsed.as_mapping().and_then(|stats| stats.tokens_saved),
        Some(5)
    );
}

#[test]
fn as_mapping_is_none_for_a_string() {
    let parsed: MappingOrScalar<Stats> =
        serde_json::from_str(r#""not-a-mapping""#).expect("a string deserializes");
    assert!(parsed.as_mapping().is_none());
}

#[test]
fn as_mapping_is_none_for_a_number() {
    let parsed: MappingOrScalar<Stats> = serde_json::from_str("5").expect("a number deserializes");
    assert!(parsed.as_mapping().is_none());
}

#[test]
fn as_mapping_is_none_for_a_same_arity_array() {
    let parsed: MappingOrScalar<Stats> =
        serde_json::from_str("[5]").expect("a one-element array deserializes");
    assert!(parsed.as_mapping().is_none());
}
