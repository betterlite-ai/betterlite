#![cfg(test)]

use rstest::rstest;

use crate::datadog_llm_privacy::datadog_budget_reset_decoder::{
    BudgetResetError, MAX_BUDGET_RESET_CHARS, decode_budget_reset,
};

#[rstest]
#[case::naive("2026-01-01T00:00:00", "2026-01-01T00:00:00+00:00")]
#[case::naive_with_fraction("2026-01-01T00:00:00.123456", "2026-01-01T00:00:00.123456+00:00")]
#[case::naive_with_a_space("2026-01-01 12:34:56", "2026-01-01T12:34:56+00:00")]
#[case::date_only("2026-01-01", "2026-01-01T00:00:00+00:00")]
#[case::zulu("2026-01-01T00:00:00Z", "2026-01-01T00:00:00+00:00")]
#[case::zulu_with_fraction("2026-01-01T00:00:00.5Z", "2026-01-01T00:00:00.500000+00:00")]
#[case::positive_offset("2026-01-01T00:00:00+02:00", "2026-01-01T00:00:00+02:00")]
#[case::negative_offset("2026-01-01T00:00:00-05:30", "2026-01-01T00:00:00-05:30")]
#[case::zero_micros("2026-01-01T00:00:00.000000", "2026-01-01T00:00:00+00:00")]
#[case::sub_microsecond("2026-01-01T00:00:00.0000001", "2026-01-01T00:00:00+00:00")]
#[case::long_fraction("2026-01-01T00:00:00.123456789012", "2026-01-01T00:00:00.123456+00:00")]
fn decodes_iso_timestamps_the_way_python_writes_them(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(decode_budget_reset(text), Ok(expected.to_owned()));
}

#[rstest]
#[case::words("next tuesday")]
#[case::truncated("2026-01-")]
#[case::empty("")]
#[case::month_thirteen("2026-13-01T00:00:00")]
#[case::no_seconds("2026-01-01T00:00")]
#[case::basic_format("20260101T000000")]
#[case::double_zulu("2026-01-01T00:00:00ZZ")]
#[case::trailing_text("2026-01-01T00:00:00+00:00 and more")]
fn rejects_a_value_that_is_not_an_iso_datetime(#[case] text: &str) {
    assert_eq!(decode_budget_reset(text), Err(BudgetResetError::Invalid));
}

fn fraction_digits(count: usize) -> String {
    let digits = "1".repeat(count);
    format!("2026-01-01T00:00:00.{digits}")
}

#[test]
fn accepts_a_timestamp_at_the_character_limit() {
    let value = fraction_digits(44);
    assert_eq!(value.chars().count(), MAX_BUDGET_RESET_CHARS);
    assert_eq!(
        decode_budget_reset(&value),
        Ok("2026-01-01T00:00:00.111111+00:00".to_owned())
    );
}

#[test]
fn refuses_a_timestamp_past_the_character_limit() {
    let value = fraction_digits(45);
    assert_eq!(
        value.chars().count(),
        MAX_BUDGET_RESET_CHARS.saturating_add(1)
    );
    assert_eq!(
        decode_budget_reset(&value),
        Err(BudgetResetError::TooLong { length: 65 })
    );
}

#[test]
fn keeps_a_utc_offset_that_is_not_zero() {
    assert_eq!(
        decode_budget_reset("2026-01-01T00:00:00.000001+05:45"),
        Ok("2026-01-01T00:00:00.000001+05:45".to_owned())
    );
}
