#![cfg(test)]
use super::time::{DateTimeInput, FixedLocalTimeResolver, to_seconds};
use chrono::{FixedOffset, NaiveDate};
use rust_decimal::Decimal;

#[test]
fn resolver_is_injected_and_datetime_timezone_is_explicit() {
    let utc = FixedOffset::east_opt(0).expect("valid offset");
    let naive = NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("valid date")
        .and_hms_opt(0, 0, 0)
        .expect("valid time");
    let datetime = DateTimeInput::Naive(naive);
    assert_eq!(
        to_seconds(
            None,
            Some(datetime),
            None,
            &FixedLocalTimeResolver { offset: utc }
        ),
        Some(Decimal::from(1_704_067_200_i64))
    );
}
