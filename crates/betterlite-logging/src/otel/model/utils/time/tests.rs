#![cfg(test)]
use super::{DateTimeInput, FixedLocalTimeResolver, LocalTimeResolver, to_ns, to_seconds};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeZone as _};
use rstest::rstest;
use rust_decimal::Decimal;

fn resolver() -> FixedLocalTimeResolver {
    FixedLocalTimeResolver {
        offset: FixedOffset::east_opt(0).expect("valid offset"),
    }
}

#[test]
fn epoch_conversion_preserves_units_and_truncates() {
    let value = Decimal::new(125, 2);
    assert_eq!(to_ns(Some(&value), None, &resolver()), Some(1_250_000_000));
    assert_eq!(
        to_seconds(Some(&value), None, None, &resolver()),
        Some(value)
    );
    let positive_fractional_nanosecond = Decimal::new(19, 10);
    let negative_fractional_nanosecond = Decimal::new(-19, 10);
    assert_eq!(
        to_ns(Some(&positive_fractional_nanosecond), None, &resolver()),
        Some(1)
    );
    assert_eq!(
        to_ns(Some(&negative_fractional_nanosecond), None, &resolver()),
        Some(-1)
    );
}

#[test]
fn formatted_time_uses_injected_local_resolution() {
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 01:00:00.5"), &resolver()),
        Some(Decimal::new(36005, 1))
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 00:00:00.123456"), &resolver()),
        Some(Decimal::new(123_456, 6))
    );
    let non_utc = FixedLocalTimeResolver {
        offset: FixedOffset::east_opt(3_600).expect("valid offset"),
    };
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 01:00:00.5"), &non_utc),
        Some(Decimal::new(5, 1))
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 01:00:00.5"), &RejectLocalTimes),
        None
    );
    let naive = NaiveDate::from_ymd_opt(1970, 1, 1)
        .expect("valid date")
        .and_hms_opt(1, 0, 0)
        .expect("valid time");
    assert_eq!(
        to_ns(None, Some(DateTimeInput::Naive(naive)), &resolver()),
        Some(3_600_000_000_000)
    );
}

#[test]
fn formatted_time_rejects_excess_precision_and_leap_seconds() {
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 00:00:00.1234567"), &resolver()),
        None
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 00:00:60"), &resolver()),
        None
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-01-01 00:00:60.1"), &resolver()),
        None
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-01-  1 00:00:00"), &resolver()),
        None
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-01- 01 00:00:00"), &resolver()),
        None
    );
    let at_limit = format!("1970-01-01{}00:00:00", " ".repeat(46));
    assert_eq!(at_limit.len(), 64);
    assert_eq!(
        to_seconds(None, None, Some(&at_limit), &resolver()),
        Some(Decimal::ZERO)
    );
    let above_limit = format!("1970-01-01{}00:00:00", " ".repeat(47));
    assert_eq!(above_limit.len(), 65);
    assert_eq!(
        to_seconds(None, None, Some(&above_limit), &resolver()),
        None
    );
}

#[test]
fn formatted_time_accepts_unpadded_date_and_clock_fields() {
    assert_eq!(
        to_seconds(None, None, Some("1970-1-1 0:0:0"), &resolver()),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        to_seconds(None, None, Some("1970-1-1 0:0:0.1"), &resolver()),
        Some(Decimal::new(1, 1))
    );
}

#[rstest]
#[case::space_padded_day("1970-01- 1 00:00:00")]
#[case::repeated_separator_space("1970-01-01  00:00:00")]
#[case::tab_separator("1970-01-01\t00:00:00")]
fn formatted_time_accepts_whitespace_variants(#[case] input: &str) {
    assert_eq!(
        to_seconds(None, None, Some(input), &resolver()),
        Some(Decimal::ZERO)
    );
}

#[test]
fn non_utc_offsets_and_dst_gap_or_fold_policy_come_from_resolver() {
    let local = NaiveDate::from_ymd_opt(2024, 3, 31)
        .expect("valid date")
        .and_hms_opt(2, 30, 0)
        .expect("valid time");
    let non_utc = FixedLocalTimeResolver {
        offset: FixedOffset::east_opt(3_600).expect("valid offset"),
    };
    assert_eq!(
        to_seconds(None, Some(DateTimeInput::Naive(local)), None, &non_utc),
        Some(Decimal::from(1_711_848_600_i64))
    );
    assert_eq!(
        to_seconds(
            None,
            Some(DateTimeInput::Naive(local)),
            None,
            &RejectLocalTimes
        ),
        None
    );
    let fold = NaiveDate::from_ymd_opt(2024, 10, 27)
        .expect("valid date")
        .and_hms_opt(2, 30, 0)
        .expect("valid time");
    let fold_resolver = OffsetResolver {
        offset: FixedOffset::east_opt(7_200).expect("valid offset"),
    };
    assert_eq!(
        to_seconds(None, Some(DateTimeInput::Naive(fold)), None, &fold_resolver),
        Some(Decimal::from(1_729_989_000_i64))
    );
}

#[test]
fn zoned_datetime_uses_its_offset_without_local_resolution() {
    let offset = FixedOffset::east_opt(7_200).expect("valid offset");
    let zoned = offset
        .with_ymd_and_hms(1970, 1, 1, 1, 0, 0)
        .single()
        .expect("valid datetime");
    assert_eq!(
        to_seconds(
            None,
            Some(DateTimeInput::Zoned(zoned)),
            None,
            &RejectLocalTimes
        ),
        Some(Decimal::from(-3_600_i64))
    );
}

struct RejectLocalTimes;

impl LocalTimeResolver for RejectLocalTimes {
    fn resolve(&self, _local: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
        None
    }
}

struct OffsetResolver {
    offset: FixedOffset,
}

impl LocalTimeResolver for OffsetResolver {
    fn resolve(&self, local: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
        self.offset.from_local_datetime(&local).single()
    }
}
