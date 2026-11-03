#![cfg(test)]
use super::{JsonValue, as_float, as_int, as_str_tuple};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[test]
fn parses_numeric_strings_and_truncates_toward_zero() {
    assert_eq!(
        as_int(&JsonValue::Number(
            serde_json::Number::from_f64(-12.9).expect("finite")
        )),
        Some(-12)
    );
    assert_eq!(as_int(&JsonValue::String("-12.9".to_owned())), None);
    assert_eq!(
        as_float(&JsonValue::String("1.25".to_owned())),
        Some(Decimal::new(125, 2))
    );
    assert_eq!(as_int(&JsonValue::Bool(true)), None);
}

#[test]
fn validates_json_shapes_for_scalars_mappings_and_tuples() {
    assert_eq!(JsonValue::Bool(false).as_bool(), Some(false));
    assert_eq!(JsonValue::String("false".to_owned()).as_bool(), None);
    let object = JsonValue::Object(BTreeMap::new());
    assert!(matches!(object, JsonValue::Object(_)));
    assert_eq!(
        as_str_tuple(&JsonValue::Array(vec![
            JsonValue::String("a".to_owned()),
            JsonValue::String("b".to_owned())
        ])),
        Some(vec!["a", "b"])
    );
    assert_eq!(
        as_str_tuple(&JsonValue::Array(vec![
            JsonValue::String("a".to_owned()),
            JsonValue::Bool(true)
        ])),
        None
    );
}
