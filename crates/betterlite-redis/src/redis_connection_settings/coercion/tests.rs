#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{coerce_bool, coerce_number, coerce_passthrough};
use crate::redis_connection_settings::model::scalars::{
    BoolOrText, FloatOrText, IntOrText, PassthroughValue,
};
use crate::redis_connection_settings::model::settings::{ClientKind, Target};

#[test]
fn coerce_int_parses_a_digit_string() {
    assert_eq!(
        coerce_number("REDIS_PORT", Some(IntOrText::Text("42".to_owned()))).unwrap(),
        Some(42)
    );
}

#[test]
fn coerce_int_rejects_an_unparsable_string() {
    assert!(
        coerce_number(
            "REDIS_PORT",
            Some(IntOrText::Text("not-a-number".to_owned()))
        )
        .is_err()
    );
}

#[test]
fn coerce_int_passes_through_an_already_typed_value() {
    assert_eq!(
        coerce_number("REDIS_PORT", Some(IntOrText::Value(7))).unwrap(),
        Some(7)
    );
}

#[test]
fn coerce_float_parses_a_decimal_string() {
    assert_eq!(
        coerce_number("REDIS_TIMEOUT", Some(FloatOrText::Text("1.5".to_owned()))).unwrap(),
        Some(1.5_f64)
    );
}

#[test]
fn coerce_float_rejects_an_unparsable_string() {
    assert!(coerce_number("REDIS_TIMEOUT", Some(FloatOrText::Text("nope".to_owned()))).is_err());
}

#[test]
fn coerce_bool_accepts_known_words_and_rejects_unknown_words() {
    assert_eq!(
        coerce_bool("REDIS_SSL", Some(BoolOrText::Text("true".to_owned()))).unwrap(),
        Some(true)
    );
    assert!(coerce_bool("REDIS_SSL", Some(BoolOrText::Text("garbage".to_owned()))).is_err());
}

#[test]
fn coerce_passthrough_rejects_a_failed_int_conversion() {
    let mut value = PassthroughValue::Text("three".to_owned());
    assert!(
        super::coerce_passthrough("cluster_error_retry_attempts", &mut value, Target::Cluster)
            .is_err()
    );
}

#[test]
fn coerce_passthrough_coerces_a_recognized_bool_name() {
    let mut passthrough = BTreeMap::new();
    passthrough.insert(
        "require_full_coverage".to_owned(),
        PassthroughValue::Text("yes".to_owned()),
    );
    coerce_options(Target::Cluster, ClientKind::Sync, &mut passthrough);
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
    coerce_options(Target::Direct, ClientKind::Sync, &mut passthrough);
    assert_eq!(
        passthrough.get("some_unmodeled_kwarg"),
        Some(&PassthroughValue::Text("3".to_owned()))
    );
}

#[rstest]
#[case::grouped("6_380")]
#[case::leading("_123")]
#[case::trailing("123_")]
#[case::doubled("6__380")]
fn coerce_numbers_reject_underscores(#[case] text: &str) {
    assert!(coerce_number("REDIS_PORT", Some(IntOrText::Text(text.to_owned()))).is_err());
    assert!(coerce_number("REDIS_TIMEOUT", Some(FloatOrText::Text(text.to_owned()))).is_err());
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
    coerce_options(Target::Direct, ClientKind::Sync, &mut passthrough);
    assert_eq!(
        passthrough["retry_on_timeout"],
        PassthroughValue::Bool(false)
    );
    assert_eq!(
        passthrough["single_connection_client"],
        PassthroughValue::Bool(true)
    );
}

fn coerce_options(
    target: Target,
    _client: ClientKind,
    options: &mut BTreeMap<String, PassthroughValue>,
) {
    options.retain(|name, value| super::coerce_passthrough(name, value, target).unwrap());
}

#[test]
fn redis_coerce_preserves_untyped_signature_parameters() {
    let mut options = BTreeMap::from([
        (
            "protocol".to_owned(),
            PassthroughValue::Text("bogus".to_owned()),
        ),
        (
            "socket_type".to_owned(),
            PassthroughValue::Text("1".to_owned()),
        ),
        (
            "auto_close_connection_pool".to_owned(),
            PassthroughValue::Text("false".to_owned()),
        ),
    ]);
    let expected = options.clone();
    coerce_options(Target::Direct, ClientKind::Sync, &mut options);
    assert_eq!(options, expected);
}

#[rstest]
#[case::health_interval("health_check_interval", "30", 30)]
#[case::read_size("socket_read_size", "32768", 0x8000)]
fn redis_coerce_signature_integers(#[case] name: &str, #[case] text: &str, #[case] expected: i64) {
    let mut value = PassthroughValue::Text(text.to_owned());
    assert!(super::coerce_passthrough(name, &mut value, Target::Direct).unwrap());
    assert_eq!(
        value,
        PassthroughValue::Number(serde_json::Number::from(expected))
    );
}

#[rstest]
#[case::direct_integer(Target::Direct, "reinitialize_steps", "5")]
#[case::url_integer(Target::Url, "cluster_error_retry_attempts", "10")]
#[case::sentinel_integer(Target::Sentinel, "reinitialize_steps", "5")]
#[case::direct_boolean(Target::Direct, "require_full_coverage", "yes")]
#[case::url_boolean(Target::Url, "read_from_replicas", "false")]
#[case::sentinel_boolean(Target::Sentinel, "dynamic_startup_nodes", "true")]
fn redis_coerce_leaves_cluster_options_untyped_elsewhere(
    #[case] target: Target,
    #[case] name: &str,
    #[case] text: &str,
) {
    let mut value = PassthroughValue::Text(text.to_owned());
    assert!(super::coerce_passthrough(name, &mut value, target).unwrap());
    assert_eq!(value, PassthroughValue::Text(text.to_owned()));
}

#[test]
fn shared_env_redis_coercion() {
    assert_eq!(
        coerce_bool("REDIS_SSL", Some(BoolOrText::Text(" Yes ".to_owned()))).unwrap(),
        Some(true)
    );
    assert_eq!(
        coerce_bool("REDIS_SSL", Some(BoolOrText::Text(String::new()))).unwrap(),
        None
    );
    let bool_error = coerce_bool(
        "REDIS_SSL",
        Some(BoolOrText::Text("maybe-secret".to_owned())),
    )
    .unwrap_err()
    .to_string();
    assert!(bool_error.contains("REDIS_SSL"));
    assert!(!bool_error.contains("maybe-secret"));
}

#[test]
fn shared_env_redis_coercion_numbers() {
    assert_eq!(
        coerce_number("REDIS_PORT", Some(IntOrText::Text(" 6380 ".to_owned()))).unwrap(),
        Some(6380)
    );
    assert_eq!(
        coerce_number("REDIS_PORT", Some(IntOrText::Text(String::new()))).unwrap(),
        None
    );
    let number_error = coerce_number("REDIS_PORT", Some(IntOrText::Text("1_000".to_owned())))
        .unwrap_err()
        .to_string();
    assert!(number_error.contains("REDIS_PORT"));
    assert!(!number_error.contains("1_000"));
    assert_eq!(
        coerce_number("REDIS_TIMEOUT", Some(FloatOrText::Text("1e3".to_owned()))).unwrap(),
        Some(1000.0)
    );
    let mut kwarg = PassthroughValue::Text("1_000".to_owned());
    let kwarg_error = coerce_passthrough("health_check_interval", &mut kwarg, Target::Direct)
        .unwrap_err()
        .to_string();
    assert!(kwarg_error.contains("REDIS_HEALTH_CHECK_INTERVAL"));
    assert!(!kwarg_error.contains("1_000"));
}
