use rstest::rstest;

use crate::passthrough::websocket::truncated_close_reason;

fn repeated(count: usize, tail: &str) -> String {
    let mut reason = "a".repeat(count);
    reason.push_str(tail);
    reason
}

#[rstest]
#[case::empty(String::new(), String::new())]
#[case::short("boom".to_owned(), "boom".to_owned())]
#[case::exactly_the_limit("a".repeat(123), "a".repeat(123))]
#[case::one_byte_over("a".repeat(124), "a".repeat(123))]
#[case::fits_a_two_byte_char(repeated(121, "\u{e9}"), repeated(121, "\u{e9}"))]
#[case::splits_a_two_byte_char(repeated(122, "\u{e9}"), "a".repeat(122))]
#[case::splits_a_four_byte_char(repeated(122, "\u{1f600}"), "a".repeat(122))]
#[case::ends_on_a_boundary(repeated(123, "\u{e9}"), "a".repeat(123))]
fn truncates_without_splitting_a_character(#[case] reason: String, #[case] expected: String) {
    assert_eq!(truncated_close_reason(&reason), expected);
}
