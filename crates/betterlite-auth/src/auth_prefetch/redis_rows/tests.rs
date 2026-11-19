use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde_json::Number;

use super::{NUMBER_KEY, value_json};

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
