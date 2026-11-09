use std::time::{Duration, Instant};

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::serialized;
use crate::datadog_llm_obs::literal::{MAX_LITERAL_DEPTH, literal_json};

fn decoded(source: &str) -> String {
    serialized(&literal_json(source).expect("the fixture is a Python literal"))
}

fn nested_lists(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

#[rstest]
#[case::single_quotes("{'a': 1}", r#"{"a":1}"#)]
#[case::double_quotes(r#"{"a": "b"}"#, r#"{"a":"b"}"#)]
#[case::list("[1, 2]", "[1,2]")]
#[case::tuple("(1, 2)", "[1,2]")]
#[case::parenthesized_value("(1)", "1")]
#[case::empty_tuple("()", "[]")]
#[case::set("{1, 2}", "[1,2]")]
#[case::keywords("(True, False, None)", "[true,false,null]")]
#[case::hexadecimal("0x1f", "31")]
#[case::octal("0o17", "15")]
#[case::binary("0b101", "5")]
#[case::integer_underscores("1_000", "1000")]
#[case::float("1.5", "1.5")]
#[case::float_trailing_dot("1.", "1.0")]
#[case::float_trailing_dot_exponent("1.e3", "1000.0")]
#[case::float_leading_zeros("01.5", "1.5")]
#[case::integer_with_underscores_in_fraction("1_0.5", "10.5")]
#[case::float_leading_dot(".5", "0.5")]
#[case::float_leading_dot_exponent(".5e3", "500.0")]
#[case::exponent("1e3", "1000.0")]
#[case::hex_underscore("0x_ff", "255")]
#[case::octal_underscore("0o_7", "7")]
#[case::binary_underscore("0b_1", "1")]
#[case::space_after_sign("- 1", "-1")]
#[case::space_after_sign_in_list("[ - 1 ]", "[-1]")]
#[case::adjacent_strings("'a' 'b'", "\"ab\"")]
#[case::adjacent_strings_parenthesized("('a' 'b')", "\"ab\"")]
#[case::adjacent_string_keys("{'a' 'b': 1}", r#"{"ab":1}"#)]
#[case::adjacent_strings_across_lines("['word '\n 'word']", r#"["word word"]"#)]
#[case::empty_set_call("set()", "[]")]
#[case::empty_set_call_with_spaces("set( )", "[]")]
#[case::empty_set_call_with_a_comment("{'a': set( # c\n)}", r#"{"a":[]}"#)]
#[case::negative("-2", "-2")]
#[case::negative_zero("-0", "0")]
#[case::nested("{'a': [1, (2, {'b': None})]}", r#"{"a":[1,[2,{"b":null}]]}"#)]
#[case::trailing_comma("[1, 2,]", "[1,2]")]
#[case::comment("1 # done", "1")]
#[case::leading_blank_lines("\n\n1", "1")]
#[case::newlines_inside_brackets("[\n  1,\n  2\n]\n", "[1,2]")]
#[case::comment_inside_brackets("[1, # one\n 2]", "[1,2]")]
#[case::triple_quoted("'''multi\nline'''", "\"multi\\nline\"")]
fn python_literals_decode_to_json(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(decoded(source), expected);
}

#[rstest]
#[case::unknown_escape(r"'\q'", "\\q")]
#[case::octal_escape(r"'\101'", "A")]
#[case::hex_escape(r"'\x41'", "A")]
#[case::newline_escape(r"'\n'", "\n")]
#[case::unicode_escape(r"'\u00e9'", "\u{e9}")]
#[case::raw_string(r"r'\n'", "\\n")]
fn escapes_decode_like_python(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(literal_json(source), Some(Json::Text(expected.to_owned())));
}

#[rstest]
#[case::unterminated_dict("{")]
#[case::unterminated_string("'unterminated")]
#[case::leading_zero("01")]
#[case::double_underscore("1__0")]
#[case::binary_expression("1+2")]
#[case::formatted_string("f'{x}'")]
#[case::bare_dot(".")]
#[case::double_sign("--1")]
#[case::exponent_without_digits("1e")]
#[case::newline_between_tokens("1\n+2")]
#[case::missing_value("[1, 2")]
#[case::missing_separator("{'a': 1, 'b'}")]
#[case::bare_base_prefix("0x")]
#[case::adjacent_bytes_and_string("'a' b'c'")]
#[case::ellipsis_name("Ellipsis")]
#[case::malformed_unicode_name(r"'\N'")]
#[case::empty_unicode_name(r"'\N{}'")]
#[case::codepoint_past_the_range(r"'\U00110000'")]
#[case::hex_with_an_imaginary_unit("0xffj")]
#[case::non_ascii_bytes("b'\u{e9}'")]
#[case::unicode_prefix_with_bytes("ub'a'")]
#[case::repeated_raw_prefix("rr'a'")]
#[case::unicode_prefix_before_raw("ur'a'")]
#[case::str_then_raw_bytes("'a' rb'c'")]
#[case::bytes_then_raw_str("b'a' r'c'")]
#[case::short_hex_escape_in_bytes(r"b'\x1'")]
#[case::empty_hex_escape_in_bytes(r"b'\x'")]
#[case::unicode_name_running_past_the_quote("{'z': '\\N{a', 'b': '}'}")]
#[case::non_ascii_space_in_a_set("{\"a\": set(\u{a0})}")]
fn unsupported_literals_decode_to_none(#[case] source: &str) {
    assert_eq!(literal_json(source), None);
}

#[rstest]
#[case::non_string_key("{1: 2}", r#"{"<unrepresentable literal>":2}"#)]
#[case::bytes("b'abc'", r#""<unrepresentable literal>""#)]
#[case::adjacent_bytes("b'a' b'b'", r#""<unrepresentable literal>""#)]
#[case::raw_bytes("rb'a'", r#""<unrepresentable literal>""#)]
#[case::raw_bytes_other_order("Rb'a'", r#""<unrepresentable literal>""#)]
#[case::bytes_keeping_a_unicode_escape(r"b'\u12'", r#""<unrepresentable literal>""#)]
#[case::bytes_with_a_wide_octal_escape(r"b'\777'", r#""<unrepresentable literal>""#)]
#[case::complex("2j", r#""<unrepresentable literal>""#)]
#[case::complex_with_a_real_part("(1+2j)", r#""<unrepresentable literal>""#)]
#[case::complex_with_a_subtracted_part("1-2j", r#""<unrepresentable literal>""#)]
#[case::complex_across_spaces("1 + 2j", r#""<unrepresentable literal>""#)]
#[case::complex_after_a_signed_number("- 1 + 2j", r#""<unrepresentable literal>""#)]
#[case::complex_with_leading_zeros("01j", r#""<unrepresentable literal>""#)]
#[case::ellipsis("...", r#""<unrepresentable literal>""#)]
#[case::overflowing_float("1e400", r#""<unrepresentable literal>""#)]
#[case::integer_past_u128(
    "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
    r#""<unrepresentable literal>""#
)]
#[case::surrogate_escape(r"'\ud83d'", "\"\u{fffd}\"")]
#[case::unicode_name_escape(r"'\N{SNOWMAN}'", r#""\\N{SNOWMAN}""#)]
fn unrepresentable_values_decode_to_a_placeholder(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(decoded(source), expected);
}

#[test]
fn a_pretty_printed_wrap_inside_a_bracket_joins_its_strings() {
    assert_eq!(
        decoded("{'a': 'word '\n         'word'}"),
        r#"{"a":"word word"}"#
    );
}

#[test]
fn a_literal_at_the_depth_cap_decodes_promptly() {
    let started = Instant::now();
    let decoded = literal_json(&nested_lists(MAX_LITERAL_DEPTH));
    assert!(decoded.is_some());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_literal_nested_past_the_cap_is_refused() {
    assert_eq!(
        literal_json(&nested_lists(MAX_LITERAL_DEPTH.saturating_add(1))),
        None
    );
}

// A parser that re-reads the last element of every list doubles its work per level; this catches that.
#[test]
fn a_literal_at_depth_twenty_decodes_well_under_a_second() {
    let started = Instant::now();
    let decoded = literal_json(&nested_lists(20));
    assert!(decoded.is_some());
    assert!(started.elapsed() < Duration::from_secs(1));
}
