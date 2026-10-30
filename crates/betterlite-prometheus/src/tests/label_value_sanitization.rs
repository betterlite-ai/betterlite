#![cfg(test)]
use betterlite_prometheus::label_sanitization::sanitize_label_value;
use rstest::rstest;

#[rstest]
#[case::missing(None, None)]
#[case::empty(Some(""), Some(""))]
#[case::plain(Some("text\u{e9} \u{1f600}"), Some("text\u{e9} \u{1f600}"))]
#[case::escapes(Some("a\\b\"c"), Some("a\\\\b\\\"c"))]
#[case::newlines(Some("a\r\nb\u{2028}c\u{2029}d"), Some("a bcd"))]
#[case::controls(Some("\t\r\n"), Some("\t "))]
#[case::literal_newline(Some("\\n"), Some("\\\\n"))]
fn translates_and_escapes(#[case] input: Option<&str>, #[case] expected: Option<&str>) {
    assert_eq!(sanitize_label_value(input).as_deref(), expected);
}
