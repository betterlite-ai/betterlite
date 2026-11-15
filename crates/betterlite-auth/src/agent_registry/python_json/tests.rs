#![cfg(test)]

use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::sorted;
use crate::agent_registry::tests::support::{ALPHA_ENTRY, BETA_ENTRY, FLOATS_ENTRY, json};

fn encoded(text: &str) -> String {
    dumps(&sorted(&json(text)))
}

#[test]
fn ascii_entries_render_like_python() {
    assert_eq!(
        encoded(ALPHA_ENTRY),
        r#"{"agent_card_params": {"model": "gpt-4o"}, "agent_name": "alpha"}"#
    );
}

#[test]
fn keys_sort_by_code_point_at_every_depth() {
    assert_eq!(
        encoded(r#"{"b":1,"a":{"z":1,"A":2},"é":3}"#),
        "{\"a\": {\"A\": 2, \"z\": 1}, \"b\": 1, \"\\u00e9\": 3}"
    );
}

#[test]
fn non_ascii_and_control_characters_use_lowercase_escapes() {
    assert_eq!(
        encoded(BETA_ENTRY),
        "{\"agent_card_params\": {\"emoji\": \"\\ud83d\\ude00\", \"mod\\u00e8le\": \"x\\u007fy\", \"raw\": \"a\\n\\t\\r\\b\\f\\\"\\\\/\"}, \"agent_name\": \"b\\u00e9ta\"}"
    );
}

#[test]
fn floats_render_as_python_repr_inside_the_entry() {
    assert_eq!(
        encoded(FLOATS_ENTRY),
        "{\"agent_card_params\": {\"a\": 1, \"b\": 1.0, \"c\": 1e+16, \"d\": 1e-05, \"e\": 1.5e-07, \"f\": 100000.0, \"g\": 0.0001, \"h\": -0.0, \"i\": 1.2345678901234567, \"j\": 5e-324, \"nested\": [true, false, null, {\"A\": [2, {\"\\u00e9\": 3}], \"z\": 1}]}, \"agent_name\": \"floats\"}"
    );
}

#[test]
fn null_and_booleans_render_as_python_literals() {
    assert_eq!(encoded("[null,true,false]"), "[null, true, false]");
}

#[test]
fn lists_keep_their_order_while_objects_sort() {
    assert_eq!(encoded(r#"[{"b":1,"a":2},3]"#), "[{\"a\": 2, \"b\": 1}, 3]");
}

#[test]
fn integers_outside_the_signed_and_unsigned_range_keep_their_digits() {
    assert_eq!(
        encoded(r#"{"v":18446744073709551616}"#),
        "{\"v\": 18446744073709551616}"
    );
}

#[test]
fn the_largest_unsigned_integer_keeps_its_digits() {
    assert_eq!(encoded("[18446744073709551615]"), "[18446744073709551615]");
}

#[rstest]
#[case("0.0", "0.0")]
#[case("-0.0", "-0.0")]
#[case("1.0", "1.0")]
#[case("-2.5", "-2.5")]
#[case("100000.0", "100000.0")]
#[case("0.0001", "0.0001")]
#[case("1000000000000000.0", "1000000000000000.0")]
#[case("1e16", "1e+16")]
#[case("1.234e16", "1.234e+16")]
#[case("1e-5", "1e-05")]
#[case("1.5e-7", "1.5e-07")]
#[case("1.2345678901234567", "1.2345678901234567")]
#[case("5e-324", "5e-324")]
#[case("1e999", "Infinity")]
#[case("-1e999", "-Infinity")]
fn float_text_follows_python_repr(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(encoded(source), expected);
}

#[test]
fn a_nan_number_renders_as_python_json_does() {
    let value = Json::Number(serde_json::Number::from_string_unchecked(String::from(
        "NaN",
    )));
    assert_eq!(dumps(&sorted(&value)), "NaN");
}
