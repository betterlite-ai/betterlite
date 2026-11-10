use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::reservation::python_text::{Style, render, str_of};

use super::support;

fn value(source: &str) -> Json {
    support::body(source).shift_remove("v").unwrap()
}

#[rstest]
#[case::defaults(
    r#"{"a": "x", "b": [true, null, 2, 1.5]}"#,
    r#"{"a": "x", "b": [true, null, 2, 1.5]}"#
)]
#[case::ensure_ascii("{\"\u{e9}\": \"x\"}", "{\"\\u00e9\": \"x\"}")]
#[case::emoji("{\"v\": \"\u{1f600}\"}", "{\"v\": \"\\ud83d\\ude00\"}")]
#[case::escapes(r#"{"v": "he said \"hi\"\n\t\\"}"#, r#"{"v": "he said \"hi\"\n\t\\"}"#)]
#[case::control(r#"{"v": "\u0008\u000c"}"#, r#"{"v": "\b\f"}"#)]
#[case::floats(
    r#"{"v": [1.5, 1e16, 1e-5, -0.0, 100.0]}"#,
    r#"{"v": [1.5, 1e+16, 1e-05, -0.0, 100.0]}"#
)]
#[case::negative_zero_integer(r#"{"v": -0}"#, r#"{"v": 0}"#)]
#[case::infinite(r#"{"v": [1e400, -1e400]}"#, r#"{"v": [Infinity, -Infinity]}"#)]
#[case::empty_containers(r#"{"v": {"a": [], "b": {}}}"#, r#"{"v": {"a": [], "b": {}}}"#)]
fn json_dumps_matches_python(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        render(&Json::Object(support::body(source)), Style::Json),
        expected
    );
}

#[rstest]
#[case::defaults(
    r#"{"a": "x", "b": [true, null, 2, 1.5]}"#,
    "{'a': 'x', 'b': [True, None, 2, 1.5]}"
)]
#[case::single_quote_key(r#"{"it's": 1}"#, r#"{"it's": 1}"#)]
#[case::double_quote_key(r#"{"say \"hi\"": 1}"#, r#"{'say "hi"': 1}"#)]
#[case::both_quotes(r#"{"both ' and \"": 1}"#, "{'both \\' and \"': 1}")]
#[case::control_characters(r#"{"v": "line\nbreak\t\u0008"}"#, r"{'v': 'line\nbreak\t\x08'}")]
#[case::separators("{\"v\": \"\u{a0} \u{2028}\"}", "{'v': '\\xa0 \\u2028'}")]
#[case::printable_unicode("{\"v\": \"h\u{e9}llo \u{1f600}\"}", "{'v': 'h\u{e9}llo \u{1f600}'}")]
#[case::infinite(r#"{"v": [1e400, -1e400]}"#, "{'v': [inf, -inf]}")]
#[case::floats(r#"{"v": [1e16, 1e-5, 100.0]}"#, "{'v': [1e+16, 1e-05, 100.0]}")]
fn repr_matches_python(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        render(&Json::Object(support::body(source)), Style::Python),
        expected
    );
}

#[test]
fn str_of_leaves_a_top_level_string_alone() {
    assert_eq!(str_of(&Json::Text("h\u{e9}llo".to_owned())), "h\u{e9}llo");
    assert_eq!(str_of(&Json::Null), "None");
    assert_eq!(str_of(&Json::Bool(true)), "True");
    assert_eq!(str_of(&value(r#"{"v": {"a": "x"}}"#)), "{'a': 'x'}");
}
