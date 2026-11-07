#![cfg(test)]

use betterlite_logging::spend_sanitize::value::SpendValue;
use serde_json::{Value, json};

use super::spec::Decoder;

fn node(value: &Value) -> SpendValue {
    let mut decoder = Decoder::default();
    decoder.decode(value).expect("a valid node spec")
}

#[test]
fn a_cycle_reference_reuses_the_declared_identity_without_content() {
    let value = node(&json!({
        "$py": "mapping",
        "id": 4_u64,
        "entries": {"self": {"$py": "ref", "id": 4_u64, "kind": "mapping"}},
    }));
    let SpendValue::Mapping(outer) = value else {
        panic!("the root is a mapping")
    };
    let Some(SpendValue::Mapping(inner)) = outer.entries.get("self") else {
        panic!("the cycle is a mapping")
    };
    assert_eq!(outer.node, inner.node);
    assert!(inner.entries.is_empty());
}

#[test]
fn a_sibling_reference_copies_the_declared_content() {
    let value = node(&json!({
        "$py": "mapping",
        "id": 1_u64,
        "entries": {
            "first": {"$py": "mapping", "id": 2_u64, "entries": {"x": 1_i64}},
            "second": {"$py": "ref", "id": 2_u64, "kind": "mapping"},
        },
    }));
    let SpendValue::Mapping(outer) = value else {
        panic!("the root is a mapping")
    };
    let Some(SpendValue::Mapping(first)) = outer.entries.get("first") else {
        panic!("first is a mapping")
    };
    let Some(SpendValue::Mapping(second)) = outer.entries.get("second") else {
        panic!("second is a mapping")
    };
    assert_eq!(first.node, second.node);
    assert_eq!(first.entries.get("x"), second.entries.get("x"));
}

#[test]
fn model_and_object_nodes_keep_their_kind() {
    let value = node(&json!({
        "$py": "mapping",
        "id": 1_u64,
        "entries": {
            "model": {"$py": "model", "id": 2_u64, "entries": {"a": 1_i64}},
            "object": {"$py": "object", "id": 3_u64, "entries": {"b": 2_i64}},
            "list": {"$py": "list", "id": 4_u64, "items": [1_i64]},
        },
    }));
    let SpendValue::Mapping(outer) = value else {
        panic!("the root is a mapping")
    };
    assert!(matches!(
        outer.entries.get("model"),
        Some(SpendValue::Model(_))
    ));
    assert!(matches!(
        outer.entries.get("object"),
        Some(SpendValue::Object(_))
    ));
    assert!(matches!(
        outer.entries.get("list"),
        Some(SpendValue::List(_))
    ));
}
