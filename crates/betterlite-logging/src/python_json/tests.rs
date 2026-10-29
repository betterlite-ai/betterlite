#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::dumps;

#[rstest]
#[case::nested(
    r#"{"b": [1, true, false, null], "a": {}}"#,
    r#"{"b": [1, true, false, null], "a": {}}"#
)]
#[case::escapes(r#""q\"b\\n\nr\rt\tb\bf\f""#, r#""q\"b\\n\nr\rt\tb\bf\f""#)]
#[case::controls(r#""\u0001\u007f~ ""#, r#""\u0001\u007f~ ""#)]
#[case::unicode("\"\u{e9}\u{1f600}\"", concat!("\"", "\\", "u00e9", "\\", "ud83d", "\\", "ude00\""))]
#[case::negative_zero("-0", "0")]
#[case::big_int("123456789012345678901234567890", "123456789012345678901234567890")]
#[case::float("1.50", "1.5")]
#[case::large_float("1e16", "1e+16")]
#[case::large_exponent("1.5E20", "1.5e+20")]
#[case::small_float("0.00001", "1e-05")]
#[case::plain_float("0.0001", "0.0001")]
#[case::infinity("1e999", "Infinity")]
#[case::negative_infinity("-1e999", "-Infinity")]
#[case::lists("[[], [1, 2]]", "[[], [1, 2]]")]
fn writes_python_json_text(#[case] source: &str, #[case] expected: &str) {
    let value: Json = serde_json::from_str(source).unwrap();
    assert_eq!(dumps(&value), expected);
}
