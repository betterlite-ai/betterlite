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
    assert_eq!(as_int(&OtelModelValue::Bool(true)), None);
}

#[test]
fn validates_json_shapes_for_scalars_mappings_and_tuples() {
    assert_eq!(OtelModelValue::Bool(false).as_bool(), Some(false));
    assert_eq!(OtelModelValue::String("false".to_owned()).as_bool(), None);
    let object = OtelModelValue::Object(BTreeMap::new());
    assert!(matches!(object, OtelModelValue::Object(_)));
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
