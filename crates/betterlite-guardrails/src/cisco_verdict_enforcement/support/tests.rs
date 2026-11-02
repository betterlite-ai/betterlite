#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_configuration::records::Direction;
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict_enforcement::support::{
    direction_label, insert_serialized, rule_name, surface_label,
};

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

#[test]
fn surface_label_matches_chat_and_mcp() {
    assert_eq!(surface_label(InspectionType::Chat), "chat");
    assert_eq!(surface_label(InspectionType::Mcp), "mcp");
}

#[test]
fn direction_label_matches_input_and_output() {
    assert_eq!(direction_label(Direction::Input), "input");
    assert_eq!(direction_label(Direction::Output), "output");
}

#[test]
fn rule_name_reads_the_rule_name_field() {
    let rule = obj(r#"{"rule_name": "pii"}"#);
    assert_eq!(rule_name(&rule), Some("pii".to_owned()));
}

#[test]
fn rule_name_is_none_when_missing_or_not_a_string() {
    assert_eq!(rule_name(&obj("{}")), None);
    assert_eq!(rule_name(&obj(r#"{"rule_name": 1}"#)), None);
}

#[test]
fn insert_serialized_writes_a_value() {
    let mut fields = Fields::new();
    insert_serialized(&mut fields, "key", &"value");
    assert_eq!(fields.get("key").map(|raw| raw.get()), Some("\"value\""));
}

#[test]
fn insert_serialized_skips_a_value_that_fails_to_serialize() {
    let mut non_string_keyed_map = BTreeMap::new();
    non_string_keyed_map.insert(vec![1_u8, 2, 3], "value");
    let mut fields = Fields::new();
    insert_serialized(&mut fields, "key", &non_string_keyed_map);
    assert!(!fields.contains_key("key"));
}

#[test]
fn insert_serialized_writes_none_as_explicit_null() {
    let mut fields = Fields::new();
    let is_safe: Option<bool> = None;
    insert_serialized(&mut fields, "is_safe", &is_safe);
    assert_eq!(fields.get("is_safe").map(|raw| raw.get()), Some("null"));
}
