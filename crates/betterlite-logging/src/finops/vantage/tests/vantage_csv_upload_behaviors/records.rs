use rstest::rstest;

use crate::finops::vantage::records::{is_blank, split_records};

#[rstest]
#[case::plain_lines("h\na\nb\n", &["h", "a", "b", ""])]
#[case::carriage_return_stays_on_the_line("h\r\na\r\n", &["h\r", "a\r", ""])]
#[case::no_trailing_newline("h\na", &["h", "a"])]
#[case::empty_input("", &[""])]
#[case::newline_inside_quotes_does_not_split("h\n\"a\nb\",c\nd\n", &["h", "\"a\nb\",c", "d", ""])]
#[case::doubled_quote_stays_inside_the_field("h\n\"a\"\"\nb\",c\n", &["h", "\"a\"\"\nb\",c", ""])]
#[case::quote_inside_an_unquoted_field_is_literal("h\n5\" pipe,x\ny\n", &["h", "5\" pipe,x", "y", ""])]
#[case::quote_after_a_comma_opens_a_field("h\nx,\"a\nb\"\ny\n", &["h", "x,\"a\nb\"", "y", ""])]
#[case::quote_after_a_space_is_literal("h\nx, \"a\nb\"\n", &["h", "x, \"a", "b\"", ""])]
#[case::unterminated_quote_runs_to_the_end("h\n\"a\nb", &["h", "\"a\nb"])]
#[case::text_after_a_closing_quote_stays_in_the_field("\"ab\"cd,e\nf", &["\"ab\"cd,e", "f"])]
fn records_split_on_unquoted_newlines(#[case] input: &str, #[case] expected: &[&str]) {
    let records: Vec<&[u8]> = split_records(input.as_bytes());
    let wanted: Vec<&[u8]> = expected.iter().map(|record| record.as_bytes()).collect();
    assert_eq!(records, wanted);
}

#[rstest]
#[case::empty(b"", true)]
#[case::spaces(b"   ", true)]
#[case::every_ascii_whitespace(b" \t\r\n\x0b\x0c", true)]
#[case::text(b" a ", false)]
#[case::non_breaking_space_is_not_ascii_whitespace("\u{a0}".as_bytes(), false)]
fn blank_records_are_ascii_whitespace_only(#[case] record: &[u8], #[case] blank: bool) {
    assert_eq!(is_blank(record), blank);
}
