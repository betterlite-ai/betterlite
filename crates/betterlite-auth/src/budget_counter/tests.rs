#![cfg(test)]
use std::collections::HashMap;

use super::{_COUNTER_ENTITY_TYPES, BudgetCounter, BudgetValue, FloatInput, IntInput, ValueLookup};

#[test]
fn structure_has_all_counter_fields() {
    assert_eq!(
        BudgetCounter {
            counter_key: "spend:key:test".to_owned(),
            max_budget: 10.0_f64,
            fallback_spend: 1.0_f64,
            entity_type: "key".to_owned(),
            entity_id: "test".to_owned(),
            source_cache_key: Some("cache".to_owned()),
            spend_log_entity_id: Some("log".to_owned()),
            window_duration: Some("1h".to_owned()),
            window_start: None,
        }
        .entity_id,
        "test"
    );
}

#[test]
fn entity_mapping_handles_all_names() {
    assert!(_COUNTER_ENTITY_TYPES.contains(&("Model access group", "model_access_group")));
}

#[test]
fn to_float_accepts_numbers_strings_and_bytes() {
    assert_eq!(FloatInput::float_value(2_i32), Some(2.0_f64));
    assert_eq!(FloatInput::float_value("2.5"), Some(2.5_f64));
    let float_bytes: &[u8] = b"3.5";
    assert_eq!(FloatInput::float_value(float_bytes), Some(3.5_f64));
}

#[test]
fn to_int_accepts_numbers_strings_and_bytes() {
    assert_eq!(IntInput::int_value(2_i32), Some(2_i64));
    assert_eq!(IntInput::int_value("25"), Some(25_i64));
    let int_bytes: &[u8] = b"35";
    assert_eq!(IntInput::int_value(int_bytes), Some(35_i64));
}

#[test]
fn get_value_reads_mapping_without_conversion() {
    let values = HashMap::from([(String::from("spend"), String::from("2.5"))]);
    assert_eq!(
        ValueLookup::value(&values, "spend"),
        Some(&String::from("2.5"))
    );
}

struct Object {
    spend: BudgetValue,
}

impl ValueLookup for Object {
    fn value(&self, key: &str) -> Option<&BudgetValue> {
        (key == "spend").then_some(&self.spend)
    }
}

#[test]
fn get_value_reads_object_attribute() {
    let object = Object {
        spend: String::from("3"),
    };
    assert_eq!(
        ValueLookup::value(&object, "spend"),
        Some(&String::from("3"))
    );
}
