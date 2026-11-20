#![cfg(test)]

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use crate::spend_queues::entity::SpendAmount;
use crate::window_queue::stamp::{
    LocalZone, WindowIncrement, WindowStamp, build_window_spend_transaction, decode_window_stamp,
};
use crate::window_queue::{WindowQueueError, WindowSpendTransaction};

struct FixedZone {
    offset: FixedOffset,
}

impl LocalZone for FixedZone {
    fn offset_for(&self, _local: NaiveDateTime) -> FixedOffset {
        self.offset
    }
}

fn zone(offset_seconds: i32) -> FixedZone {
    FixedZone {
        offset: FixedOffset::east_opt(offset_seconds).expect("a test offset"),
    }
}

fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").expect("a test stamp")
}

fn aware(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).expect("a test stamp")
}

fn increment(
    window_start: WindowStamp,
    started_at: Option<WindowStamp>,
) -> WindowIncrement<'static> {
    WindowIncrement {
        entity_type: "key",
        entity_id: "key-1",
        window_duration: "30d",
        window_start,
        spend: Decimal::new(25, 2),
        started_at,
    }
}

#[rstest]
#[case::utc("2026-08-01T00:00:00Z")]
#[case::colon_offset("2026-08-01T00:00:00+02:00")]
#[case::space_and_compact_offset("2026-08-01 03:04:05.123456+0200")]
fn decode_reads_an_offset_stamp(#[case] text: &str) {
    let decoded = decode_window_stamp(text.as_bytes()).expect("an aware stamp");
    assert!(matches!(decoded, WindowStamp::Aware(_)));
}

#[rstest]
#[case::fraction("2026-08-01T03:04:05.123456")]
#[case::space("2026-08-01 03:04:05")]
#[case::minutes("2026-08-01T03:04")]
#[case::minutes_and_space("2026-08-01 03:04")]
fn decode_reads_a_naive_stamp(#[case] text: &str) {
    let decoded = decode_window_stamp(text.as_bytes()).expect("a naive stamp");
    assert!(matches!(decoded, WindowStamp::Naive(_)));
}

#[test]
fn decode_keeps_the_wall_clock_of_each_naive_shape() {
    assert_eq!(
        decode_window_stamp(b"2026-08-01T03:04:05.123456").expect("a naive stamp"),
        WindowStamp::Naive(naive("2026-08-01T03:04:05.123456"))
    );
    assert_eq!(
        decode_window_stamp(b"2026-08-01 03:04:05").expect("a naive stamp"),
        WindowStamp::Naive(naive("2026-08-01T03:04:05"))
    );
    assert_eq!(
        decode_window_stamp(b"2026-08-01T03:04").expect("a naive stamp"),
        WindowStamp::Naive(naive("2026-08-01T03:04:00"))
    );
    assert_eq!(
        decode_window_stamp(b"2026-08-01").expect("a bare date"),
        WindowStamp::Naive(
            NaiveDate::from_ymd_opt(2026, 8, 1)
                .expect("a test date")
                .and_time(NaiveTime::MIN)
        )
    );
}

#[test]
fn decode_reads_the_offset_stamp_as_the_same_instant() {
    let decoded = decode_window_stamp(b"2026-08-01 03:04:05+02:00").expect("an aware stamp");
    let expected = Utc
        .with_ymd_and_hms(2026, 8, 1, 1, 4, 5)
        .single()
        .expect("a test instant")
        .fixed_offset();
    assert_eq!(decoded, WindowStamp::Aware(expected));
}

#[test]
fn decode_rejects_text_that_is_not_a_stamp() {
    assert!(matches!(
        decode_window_stamp(b"not a stamp"),
        Err(WindowQueueError::InvalidStamp { .. })
    ));
    assert!(matches!(
        decode_window_stamp(b"2026-13-45"),
        Err(WindowQueueError::InvalidStamp { .. })
    ));
    assert!(matches!(
        decode_window_stamp(b"\xff\xfe"),
        Err(WindowQueueError::InvalidStampText { .. })
    ));
}

#[test]
fn an_aware_window_start_and_start_become_naive_utc() {
    let transaction = build_window_spend_transaction(
        &increment(
            WindowStamp::Aware(aware("2026-08-01T00:30:00+02:00")),
            Some(WindowStamp::Aware(aware("2026-08-01T00:30:00.250000Z"))),
        ),
        &zone(0),
    )
    .expect("the increment builds");
    assert_eq!(transaction.window_start, "2026-07-31T22:30:00.000000");
    assert_eq!(
        transaction.started_at.as_deref(),
        Some("2026-08-01T00:30:00.250000")
    );
    assert_eq!(transaction.spend, SpendAmount(Decimal::new(25, 2)));
}

#[test]
fn a_naive_window_start_keeps_its_wall_clock() {
    let transaction = build_window_spend_transaction(
        &increment(
            WindowStamp::Naive(naive("2026-08-01T12:34:56.123456")),
            None,
        ),
        &zone(7200),
    )
    .expect("the increment builds");
    assert_eq!(transaction.window_start, "2026-08-01T12:34:56.123456");
    assert_eq!(transaction.started_at, None);
}

#[test]
fn a_naive_start_is_read_in_the_host_zone() {
    let east = build_window_spend_transaction(
        &increment(
            WindowStamp::Naive(naive("2026-08-01T12:34:56")),
            Some(WindowStamp::Naive(naive("2026-08-01T12:34:56"))),
        ),
        &zone(7200),
    )
    .expect("the increment builds");
    assert_eq!(
        east.started_at.as_deref(),
        Some("2026-08-01T10:34:56.000000")
    );
    let west = build_window_spend_transaction(
        &increment(
            WindowStamp::Naive(naive("2026-08-01T12:34:56")),
            Some(WindowStamp::Naive(naive("2026-08-01T12:34:56"))),
        ),
        &zone(-18000),
    )
    .expect("the increment builds");
    assert_eq!(
        west.started_at.as_deref(),
        Some("2026-08-01T17:34:56.000000")
    );
}

#[test]
fn a_start_outside_the_calendar_range_fails() {
    let outcome = build_window_spend_transaction(
        &increment(
            WindowStamp::Naive(naive("2026-08-01T00:00:00")),
            Some(WindowStamp::Naive(NaiveDateTime::MAX)),
        ),
        &zone(-7200),
    );
    assert!(matches!(outcome, Err(WindowQueueError::StampOutOfRange)));
}

#[test]
fn the_wire_transaction_keeps_its_six_fields() {
    let transaction = build_window_spend_transaction(
        &increment(WindowStamp::Aware(aware("2026-08-01T00:00:00Z")), None),
        &zone(0),
    )
    .expect("the increment builds");
    assert_eq!(
        transaction,
        WindowSpendTransaction {
            entity_type: String::from("key"),
            entity_id: String::from("key-1"),
            window_duration: String::from("30d"),
            window_start: String::from("2026-08-01T00:00:00.000000"),
            spend: SpendAmount(Decimal::new(25, 2)),
            started_at: None,
        }
    );
}
