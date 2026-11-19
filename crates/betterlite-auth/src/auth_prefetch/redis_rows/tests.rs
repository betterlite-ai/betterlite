#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde_json::Number;

use super::{NUMBER_KEY, rows_json, value_json};

fn private_number(text: &str) -> CachedValue {
    let mut fields = BTreeMap::new();
    fields.insert(NUMBER_KEY.to_owned(), CachedValue::Text(text.to_owned()));
    CachedValue::Object(fields)
}

fn expects_float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the fixture float is finite"))
}

#[test]
fn arbitrary_precision_float_map_reads_as_a_number() {
    assert_eq!(value_json(Some(private_number("1.5"))), expects_float(1.5));
}

#[test]
fn private_key_object_with_extra_fields_stays_an_object() {
    let mut fields = BTreeMap::new();
    fields.insert(NUMBER_KEY.to_owned(), CachedValue::Text("1.5".to_owned()));
    fields.insert("other".to_owned(), CachedValue::Boolean(true));
    let Json::Object(converted) = value_json(Some(CachedValue::Object(fields))) else {
        panic!("the two-key map reads as an object");
    };
    assert_eq!(converted.get("other"), Some(&Json::Bool(true)));
}

#[test]
fn single_other_key_map_stays_an_object() {
    let mut fields = BTreeMap::new();
    fields.insert("other".to_owned(), CachedValue::Text("1.5".to_owned()));
    let Json::Object(converted) = value_json(Some(CachedValue::Object(fields))) else {
        panic!("a single non-number key reads as an object");
    };
    assert_eq!(converted.get("other"), Some(&Json::Text("1.5".to_owned())));
}

#[test]
fn rows_json_builds_the_keyed_map() {
    let mut found = BTreeMap::new();
    found.insert(
        "user-1".to_owned(),
        Some(CachedValue::Text("row".to_owned())),
    );
    found.insert("missing".to_owned(), None);
    let Json::Object(fields) = rows_json(found) else {
        panic!("the read builds an object");
    };
    assert_eq!(fields.get("user-1"), Some(&Json::Text("row".to_owned())));
    assert_eq!(fields.get("missing"), Some(&Json::Null));
}
