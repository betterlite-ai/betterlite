#![cfg(test)]

use std::collections::BTreeMap;

use super::{coerce_bool, coerce_float, coerce_int, coerce_passthrough};
use crate::redis_connection_settings::model::scalars::{
    BoolOrText, FloatOrText, IntOrText, PassthroughValue,
};
use crate::redis_connection_settings::model::settings::{ClientKind, Target};

#[test]
fn coerce_int_parses_a_digit_string() {
    assert_eq!(coerce_int(Some(IntOrText::Text("42".to_owned()))), Some(42));
}

#[test]
fn coerce_int_drops_an_unparsable_string() {
    assert_eq!(
        coerce_int(Some(IntOrText::Text("not-a-number".to_owned()))),
        None
    );
}

#[test]
fn coerce_int_passes_through_an_already_typed_value() {
    assert_eq!(coerce_int(Some(IntOrText::Int(7))), Some(7));
}

#[test]
fn coerce_float_parses_a_decimal_string() {
    assert_eq!(
        coerce_float(Some(FloatOrText::Text("1.5".to_owned()))),
        Some(1.5_f64)
    );
}

#[test]
fn coerce_float_drops_an_unparsable_string() {
    assert_eq!(
        coerce_float(Some(FloatOrText::Text("nope".to_owned()))),
        None
    );
}

#[test]
fn coerce_bool_never_fails() {
    assert_eq!(
        coerce_bool(Some(BoolOrText::Text("true".to_owned()))),
        Some(true)
    );
    assert_eq!(
        coerce_bool(Some(BoolOrText::Text("garbage".to_owned()))),
        Some(false)
    );
}

#[test]
fn coerce_passthrough_drops_a_failed_int_conversion() {
    let mut passthrough = BTreeMap::new();
    passthrough.insert(
        "cluster_error_retry_attempts".to_owned(),
        PassthroughValue::Text("three".to_owned()),
    );
    coerce_passthrough(Target::Cluster, ClientKind::Sync, &mut passthrough);
    assert!(!passthrough.contains_key("cluster_error_retry_attempts"));
}

#[test]
fn coerce_passthrough_coerces_a_recognized_bool_name() {
    let mut passthrough = BTreeMap::new();
    passthrough.insert(
        "require_full_coverage".to_owned(),
        PassthroughValue::Text("yes".to_owned()),
    );
    coerce_passthrough(Target::Cluster, ClientKind::Sync, &mut passthrough);
    assert_eq!(
        passthrough.get("require_full_coverage"),
        Some(&PassthroughValue::Bool(true))
    );
}

#[test]
fn coerce_passthrough_leaves_an_unrecognized_name_untouched() {
    let mut passthrough = BTreeMap::new();
    passthrough.insert(
        "some_unmodeled_kwarg".to_owned(),
        PassthroughValue::Text("3".to_owned()),
    );
    coerce_passthrough(Target::Direct, ClientKind::Sync, &mut passthrough);
    assert_eq!(
        passthrough.get("some_unmodeled_kwarg"),
        Some(&PassthroughValue::Text("3".to_owned()))
    );
}

#[test]
fn coerce_numbers_accept_whitespace_and_digit_separators() {
    assert_eq!(
        coerce_int(Some(IntOrText::Text(" 6_380\n".to_owned()))),
        Some(6380)
    );
    assert_eq!(
        coerce_float(Some(FloatOrText::Text(" 1_5.0 ".to_owned()))),
        Some(15.0_f64)
    );
}

#[test]
fn coerce_signature_boolean_names() {
    let mut passthrough = BTreeMap::from([
        (
            "retry_on_timeout".to_owned(),
            PassthroughValue::Text("false".to_owned()),
        ),
        (
            "single_connection_client".to_owned(),
            PassthroughValue::Text("true".to_owned()),
        ),
    ]);
    coerce_passthrough(Target::Direct, ClientKind::Sync, &mut passthrough);
    assert_eq!(
        passthrough["retry_on_timeout"],
        PassthroughValue::Bool(false)
    );
    assert_eq!(
        passthrough["single_connection_client"],
        PassthroughValue::Bool(true)
    );
}
