#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::get_response_time;
use crate::fields::timestamps::TimestampError;

fn seconds(value: i64, scale: u32) -> Decimal {
    Decimal::new(value, scale)
}

#[rstest]
#[case::non_stream_uses_the_end(
    seconds(17_040_672_000, 1),
    seconds(17_040_672_015, 1),
    seconds(17_040_672_005, 1),
    false,
    seconds(15, 1)
)]
#[case::stream_uses_the_first_token(
    seconds(17_040_672_000, 1),
    seconds(17_040_672_015, 1),
    seconds(17_040_672_005, 1),
    true,
    seconds(5, 1)
)]
#[case::equal_times_give_zero(seconds(10, 0), seconds(10, 0), seconds(10, 0), false, Decimal::ZERO)]
fn logging_metadata_behavior_tests_behavior(
    #[case] start: Decimal,
    #[case] end: Decimal,
    #[case] completion_start: Decimal,
    #[case] stream: bool,
    #[case] expected: Decimal,
) {
    let response_time =
        get_response_time(start, end, completion_start, stream).expect("response time");
    assert_eq!(response_time, expected);
}

#[test]
fn a_difference_that_overflows_the_decimal_range_is_refused() {
    let error =
        get_response_time(Decimal::MIN, Decimal::MAX, Decimal::ZERO, false).expect_err("overflow");
    assert_eq!(error, TimestampError::ResponseTimeOverflow);
}
