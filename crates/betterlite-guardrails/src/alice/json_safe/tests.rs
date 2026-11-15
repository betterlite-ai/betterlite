#![cfg(test)]

use std::collections::BTreeMap;
use std::ptr;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{CREDENTIAL_KEYS_TO_STRIP, MAX_ITEMS, json_safe};

#[test]
fn alice_guardrail_json_safe_drops_an_object_at_the_depth_limit() {
    let leaf = GuardrailValue::Object(BTreeMap::from([(
        "key".to_owned(),
        GuardrailValue::Text("leaf".to_owned()),
    )]));
    assert_eq!(
        safe(&wrapped_object(leaf, 12), &[]),
        wrapped_object(GuardrailValue::Null, 12)
    );
}

#[test]
fn alice_guardrail_json_safe_drops_a_container_at_the_depth_limit() {
    let value = nested(
        12,
        GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]),
    );
    assert_eq!(safe(&value, &[]), nested(12, GuardrailValue::Null));
}

#[test]
fn alice_guardrail_json_safe_keeps_a_container_just_inside_the_limit() {
    let value = nested(
        11,
        GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]),
    );
    assert_eq!(safe(&value, &[]), value);
}

#[test]
fn alice_guardrail_json_safe_drops_a_revisited_value() {
    let value = GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]);
    let mut seen = vec![ptr::from_ref(&value)];
    assert_eq!(json_safe(&value, 0, &[], &mut seen), GuardrailValue::Null);
}

#[test]
fn alice_guardrail_json_safe_caps_each_container_at_five_thousand_items() {
    let items = (0..5001)
        .map(GuardrailValue::SignedInteger)
        .collect::<Vec<_>>();
    let kept = safe(&GuardrailValue::Array(items), &[]);
    match kept {
        GuardrailValue::Array(values) => {
            assert_eq!(values.len(), MAX_ITEMS);
            assert_eq!(values.last(), Some(&GuardrailValue::SignedInteger(4999)));
        }
        other => panic!("expected an array, got {other:?}"),
    }
}

#[test]
fn alice_guardrail_json_safe_strips_credential_keys_at_any_depth() {
    let stripped = safe(&credential_value(), &CREDENTIAL_KEYS_TO_STRIP);
    match stripped {
        GuardrailValue::Object(fields) => {
            assert!(fields.get("headers").is_none());
            match fields.get("outer") {
                Some(GuardrailValue::Array(items)) => match items.first() {
                    Some(GuardrailValue::Object(inner)) => {
                        assert!(inner.get("headers").is_none());
                        assert!(inner.get("provider_specific_header").is_none());
                        assert_eq!(
                            inner.get("keep"),
                            Some(&GuardrailValue::Text("kept".to_owned()))
                        );
                    }
                    other => panic!("expected an object, got {other:?}"),
                },
                other => panic!("expected an array, got {other:?}"),
            }
        }
        other => panic!("expected an object, got {other:?}"),
    }
}

fn credential_value() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "outer".to_owned(),
            GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([
                (
                    "headers".to_owned(),
                    GuardrailValue::Text("dropped".to_owned()),
                ),
                (
                    "provider_specific_header".to_owned(),
                    GuardrailValue::Text("dropped".to_owned()),
                ),
                ("keep".to_owned(), GuardrailValue::Text("kept".to_owned())),
            ]))]),
        ),
        (
            "headers".to_owned(),
            GuardrailValue::Text("dropped".to_owned()),
        ),
    ]))
}

fn safe(value: &GuardrailValue, strip_keys: &[&str]) -> GuardrailValue {
    let mut seen = Vec::new();
    json_safe(value, 0, strip_keys, &mut seen)
}

fn wrapped_object(inner: GuardrailValue, depth: usize) -> GuardrailValue {
    let mut value = inner;
    for _ in 0..depth {
        value = GuardrailValue::Object(BTreeMap::from([("key".to_owned(), value)]));
    }
    value
}

fn nested(depth: usize, leaf: GuardrailValue) -> GuardrailValue {
    let mut value = leaf;
    for _ in 0..depth {
        value = GuardrailValue::Array(vec![value]);
    }
    value
}
