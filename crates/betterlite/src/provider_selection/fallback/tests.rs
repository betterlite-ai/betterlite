#![cfg(test)]

use std::collections::BTreeSet;
use std::io::{Cursor, ErrorKind};

use rstest::rstest;

use super::{ParseOutcome, parse_selection, select_fallback};

#[rstest]
#[case::single_number("1\n", ParseOutcome::Valid(BTreeSet::from([0])))]
#[case::duplicates_and_out_of_order_are_deduped_and_sorted(
    "1,2,1,3\n",
    ParseOutcome::Valid(BTreeSet::from([0, 1, 2]))
)]
#[case::space_separated("1 2\n", ParseOutcome::Valid(BTreeSet::from([0, 1])))]
#[case::empty_input("\n", ParseOutcome::Empty)]
#[case::blank_input("   \n", ParseOutcome::Empty)]
#[case::zero_is_out_of_range("0\n", ParseOutcome::OutOfRange)]
#[case::number_above_the_catalog_is_out_of_range("99\n", ParseOutcome::OutOfRange)]
#[case::non_numeric_is_invalid("abc\n", ParseOutcome::Invalid)]
fn parses_numeric_selections(#[case] input: &str, #[case] expected: ParseOutcome) {
    assert_eq!(parse_selection(input), expected);
}

#[test]
fn deduplicates_and_sorts_valid_provider_indices() {
    let mut reader = Cursor::new(b"3,1,2,1\n".to_vec());
    let mut writer = Vec::new();

    let selected = select_fallback(&mut reader, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([0, 1, 2]));
}

#[test]
fn retries_after_empty_input_and_then_accepts_a_valid_selection() {
    let mut reader = Cursor::new(b"\n1\n".to_vec());
    let mut writer = Vec::new();

    let selected = select_fallback(&mut reader, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([0]));
    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(output.contains("Please select at least one provider."));
}

#[test]
fn retries_after_an_out_of_range_number() {
    let mut reader = Cursor::new(b"99\n2\n".to_vec());
    let mut writer = Vec::new();

    let selected = select_fallback(&mut reader, &mut writer, false).expect("selection succeeds");

    assert_eq!(selected, BTreeSet::from([1]));
    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert!(output.contains("Enter numbers between 1 and 6."));
}

#[test]
fn eof_returns_instead_of_reprompting_forever() {
    let mut reader = Cursor::new(Vec::new());
    let mut writer = Vec::new();
    let error = select_fallback(&mut reader, &mut writer, false).expect_err("EOF cancels input");
    assert!(
        matches!(&error, super::FallbackError::Io(source) if source.kind() == ErrorKind::UnexpectedEof)
    );
}
