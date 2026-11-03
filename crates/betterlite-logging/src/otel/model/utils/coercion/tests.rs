#![cfg(test)]
use super::{OtelModelValue, as_int, as_str_tuple};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[test]
fn parses_numeric_strings_and_truncates_toward_zero() {
    assert_eq!(
        as_int(&OtelModelValue::Number(
            serde_json::Number::from_f64(-12.9).expect("finite")
        )),
        Some(-12)
    );
    assert_eq!(as_int(&OtelModelValue::String("-12.9".to_owned())), None);
    assert_eq!(
        OtelModelValue::String("1.25".to_owned()).as_decimal(),
        Some(Decimal::new(125, 2))
    );
    assert_eq!(as_int(&OtelModelValue::Bool(true)), Some(1));
    assert_eq!(as_int(&OtelModelValue::Bool(false)), Some(0));
    assert_eq!(OtelModelValue::Bool(true).as_decimal(), Some(Decimal::ONE));
    assert_eq!(
        OtelModelValue::Bool(false).as_decimal(),
        Some(Decimal::ZERO)
    );
}

#[test]
fn numeric_strings_allow_surrounding_whitespace_and_digit_separators() {
    assert_eq!(as_int(&OtelModelValue::String(" 42 ".to_owned())), Some(42));
    assert_eq!(
        as_int(&OtelModelValue::String("1_000".to_owned())),
        Some(1_000)
    );
    assert_eq!(
        OtelModelValue::String(" 1.25 ".to_owned()).as_decimal(),
        Some(Decimal::new(125, 2))
    );
    assert_eq!(as_int(&OtelModelValue::String("1__000".to_owned())), None);
}

#[test]
fn validates_json_shapes_for_scalars_mappings_and_tuples() {
    assert_eq!(OtelModelValue::Bool(false).as_bool(), Some(false));
    assert_eq!(OtelModelValue::Bool(true).as_bool(), Some(true));
    assert_eq!(OtelModelValue::String("false".to_owned()).as_bool(), None);
    assert_eq!(
        OtelModelValue::Number(serde_json::Number::from(0_i64)).as_bool(),
        None
    );
    let object = OtelModelValue::Object(BTreeMap::from([(
        "key".to_owned(),
        OtelModelValue::Number(serde_json::Number::from(3_i64)),
    )]));
    assert!(
        object
            .as_object()
            .is_some_and(|map| map.contains_key("key"))
    );
    assert!(OtelModelValue::Array(Vec::new()).as_object().is_none());
    assert_eq!(
        as_str_tuple(&OtelModelValue::Array(vec![
            OtelModelValue::String("a".to_owned()),
            OtelModelValue::String("b".to_owned())
        ])),
        Some(vec!["a", "b"])
    );
    assert_eq!(
        as_str_tuple(&OtelModelValue::Array(vec![
            OtelModelValue::String("a".to_owned()),
            OtelModelValue::Bool(true)
        ])),
        None
    );
}
