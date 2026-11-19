#![cfg(test)]

use rstest::rstest;

use crate::finops::mavvrik::export::DEFAULT_MAX_ROWS;
use crate::finops::mavvrik::support::{Recorder, build, environment, try_build};

#[test]
fn an_unset_row_cap_is_five_hundred_thousand() {
    let logger = build(&environment(&[]), &Recorder::default(), None);
    assert_eq!(DEFAULT_MAX_ROWS, 500_000);
    assert_eq!(logger.max_rows, DEFAULT_MAX_ROWS);
}

#[rstest]
#[case::empty("", 500_000)]
#[case::blank("   ", 500_000)]
#[case::plain("123", 123)]
#[case::padded(" 77 ", 77)]
#[case::zero("0", 0)]
#[case::negative("-5", -5)]
fn a_configured_row_cap_is_read_as_an_integer(#[case] value: &str, #[case] expected: i64) {
    let logger = build(
        &environment(&[("MAVVRIK_FOCUS_MAX_ROWS", value)]),
        &Recorder::default(),
        None,
    );
    assert_eq!(logger.max_rows, expected);
}

#[rstest]
#[case::text("many")]
#[case::fraction("1.5")]
#[case::exponent("1e3")]
fn an_unparseable_row_cap_fails_construction(#[case] value: &str) {
    let error = try_build(
        &environment(&[("MAVVRIK_FOCUS_MAX_ROWS", value)]),
        &Recorder::default(),
        None,
    )
    .err()
    .expect("an unparseable row cap fails construction");
    assert!(error.to_string().contains("MAVVRIK_FOCUS_MAX_ROWS"));
}
