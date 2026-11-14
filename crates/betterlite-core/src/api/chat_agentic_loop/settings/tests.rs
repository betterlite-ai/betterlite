#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::{
    AgenticSettings, SettingsError, converted_stream_requested, fingerprint_text, read_settings,
};
use crate::api::chat_agentic_loop::tests::support::json_map;

fn read(entries: &[(&str, &str)]) -> AgenticSettings {
    read_settings(&json_map(entries)).unwrap()
}

#[test]
fn the_defaults_apply_when_the_keys_are_absent() {
    let settings = read(&[]);
    assert_eq!(settings.depth, 0);
    assert_eq!(settings.max_loops, 3);
    assert!(settings.fingerprints.is_empty());
}

#[rstest]
#[case::json_integer("3", 3)]
#[case::digit_string(r#""3""#, 3)]
#[case::signed_string(r#""+7""#, 7)]
fn the_depth_reads_a_whole_number(#[case] value: &str, #[case] expected: u32) {
    assert_eq!(read(&[("_agentic_loop_depth", value)]).depth, expected);
}

#[rstest]
#[case::float("1.5")]
#[case::exponent("1e3")]
#[case::boolean("true")]
#[case::null("null")]
#[case::object("{}")]
#[case::list("[]")]
fn a_depth_of_another_json_shape_is_zero(#[case] value: &str) {
    assert_eq!(read(&[("_agentic_loop_depth", value)]).depth, 0);
}

#[test]
fn a_depth_string_that_is_not_a_number_is_an_error() {
    let error = read_settings(&json_map(&[("_agentic_loop_depth", r#""abc""#)])).unwrap_err();
    assert_eq!(
        error,
        SettingsError::NotNumber {
            value: "abc".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "_agentic_loop_depth must be a whole number: abc"
    );
}

#[rstest]
#[case::negative("-1")]
#[case::past_u32("4294967296")]
#[case::past_i64("9223372036854775808")]
fn a_depth_outside_u32_is_out_of_range(#[case] value: &str) {
    let error = read_settings(&json_map(&[("_agentic_loop_depth", value)])).unwrap_err();
    assert!(matches!(&error, SettingsError::OutOfRange { .. }));
    assert_eq!(
        error.to_string(),
        format!("_agentic_loop_depth is out of range: {value}")
    );
}

#[rstest]
#[case::integer("5", 5)]
#[case::digit_string(r#""5""#, 5)]
#[case::whole_float("2.0", 2)]
#[case::null("null", 3)]
fn the_max_loops_reads_the_validated_ceiling(#[case] value: &str, #[case] expected: u32) {
    assert_eq!(read(&[("max_agentic_loops", value)]).max_loops, expected);
}

#[rstest]
#[case::boolean("true")]
#[case::word(r#""three""#)]
#[case::fraction("1.5")]
#[case::list("[]")]
fn the_max_loops_rejects_a_non_whole_value(#[case] value: &str) {
    let error = read_settings(&json_map(&[("max_agentic_loops", value)])).unwrap_err();
    assert_eq!(
        error.to_string(),
        "litellm_params.max_agentic_loops must be a whole number"
    );
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
fn the_max_loops_rejects_a_ceiling_below_one(#[case] value: &str) {
    let error = read_settings(&json_map(&[("max_agentic_loops", value)])).unwrap_err();
    assert_eq!(
        error.to_string(),
        "litellm_params.max_agentic_loops must be at least 1"
    );
}

#[test]
fn the_fingerprints_read_every_element_as_text() {
    let settings = read(&[(
        "_agentic_loop_fingerprints",
        r#"["first",1,true,{"b":2,"a":[null]}]"#,
    )]);
    assert_eq!(
        settings.fingerprints,
        ["first", "1", "true", r#"{"a":[null],"b":2}"#]
    );
}

#[test]
fn the_fingerprint_text_renders_every_shape() {
    assert_eq!(fingerprint_text(&Json::Null).unwrap(), "null");
    assert_eq!(fingerprint_text(&Json::Bool(true)).unwrap(), "true");
    assert_eq!(fingerprint_text(&Json::Bool(false)).unwrap(), "false");
    assert_eq!(
        fingerprint_text(&Json::Number(Number::from(7))).unwrap(),
        "7"
    );
    assert_eq!(
        fingerprint_text(&Json::Number(Number::from_f64(1.5).unwrap())).unwrap(),
        "1.5"
    );
    assert_eq!(
        fingerprint_text(&Json::List(vec![Json::Bool(false), Json::Null])).unwrap(),
        "[false,null]"
    );
    let object = Json::Object(
        [
            ("z".to_owned(), Json::Null),
            ("a".to_owned(), Json::Bool(true)),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(fingerprint_text(&object).unwrap(), r#"{"a":true,"z":null}"#);
}

#[rstest]
#[case::null("null")]
#[case::text(r#""text""#)]
#[case::object("{}")]
#[case::number("3")]
fn fingerprints_of_a_non_list_are_empty(#[case] value: &str) {
    assert!(
        read(&[("_agentic_loop_fingerprints", value)])
            .fingerprints
            .is_empty()
    );
}

#[rstest]
#[case::boolean("true", true)]
#[case::number("1", true)]
#[case::negative("-1", true)]
#[case::text(r#""on""#, true)]
#[case::filled_list("[0]", true)]
#[case::filled_object(r#"{"a":0}"#, true)]
#[case::false_boolean("false", false)]
#[case::zero("0", false)]
#[case::float_zero("0.0", false)]
#[case::empty_text(r#""""#, false)]
#[case::null("null", false)]
#[case::empty_list("[]", false)]
#[case::empty_object("{}", false)]
fn the_converted_stream_flag_uses_python_truthiness(#[case] value: &str, #[case] expected: bool) {
    let kwargs = json_map(&[("_code_interpreter_interception_converted_stream", value)]);
    assert_eq!(converted_stream_requested(&kwargs), expected);
}

#[test]
fn either_converted_stream_key_turns_the_flag_on() {
    assert!(converted_stream_requested(&json_map(&[(
        "_headroom_interception_converted_stream",
        "true",
    )])));
    assert!(!converted_stream_requested(&json_map(&[("other", "true")])));
    assert!(converted_stream_requested(&json_map(&[
        ("_headroom_interception_converted_stream", "false"),
        ("_code_interpreter_interception_converted_stream", "1"),
    ])));
}
