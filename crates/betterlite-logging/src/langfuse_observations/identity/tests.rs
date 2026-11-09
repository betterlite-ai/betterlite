#![cfg(test)]

use chrono::{TimeZone as _, Utc};
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::trace::IdGenerator;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{Timestamp, resolve_observation_id, resolve_trace_id, to_unix_nanos};

#[derive(Clone, Copy, Debug)]
enum Check {
    SecondsToNanos,
    SecondsTruncateTowardZero,
    DatetimeToNanos,
    AbsentTimeStaysAbsent,
    UuidPassesThrough,
    UppercaseUuidNormalizes,
    OtherTextHashes,
    EmptyTraceIdIsFresh,
    AbsentTraceIdIsFresh,
    AbsentObservationIdStaysAbsent,
    CanonicalObservationIdPassesThrough,
    UppercaseObservationIdNormalizes,
    OtherTextHashesToObservationId,
    AllZeroObservationIdHashes,
    TraceIdLengthHashesAsObservationId,
}

#[derive(Clone, Copy, Debug)]
struct FixedGenerator;

impl IdGenerator for FixedGenerator {
    fn new_trace_id(&self) -> TraceId {
        TraceId::from_hex("0f0e0d0c0b0a09080706050403020100").unwrap()
    }

    fn new_span_id(&self) -> SpanId {
        SpanId::from_hex("0f0e0d0c0b0a0908").unwrap()
    }
}

#[rstest]
#[case::seconds_convert_to_nanos(Check::SecondsToNanos)]
#[case::seconds_truncate_toward_zero(Check::SecondsTruncateTowardZero)]
#[case::datetime_converts_to_nanos(Check::DatetimeToNanos)]
#[case::absent_time_stays_absent(Check::AbsentTimeStaysAbsent)]
#[case::uuid_passes_through(Check::UuidPassesThrough)]
#[case::uppercase_uuid_normalizes(Check::UppercaseUuidNormalizes)]
#[case::other_text_hashes(Check::OtherTextHashes)]
#[case::empty_trace_id_is_fresh(Check::EmptyTraceIdIsFresh)]
#[case::absent_trace_id_is_fresh(Check::AbsentTraceIdIsFresh)]
#[case::absent_observation_id_stays_absent(Check::AbsentObservationIdStaysAbsent)]
#[case::canonical_observation_id_passes_through(Check::CanonicalObservationIdPassesThrough)]
#[case::uppercase_observation_id_normalizes(Check::UppercaseObservationIdNormalizes)]
#[case::other_text_hashes_to_observation_id(Check::OtherTextHashesToObservationId)]
#[case::all_zero_observation_id_hashes(Check::AllZeroObservationIdHashes)]
#[case::trace_id_length_hashes_as_observation_id(Check::TraceIdLengthHashesAsObservationId)]
fn observation_attributes_tests_behavior(#[case] check: Check) {
    let generator = FixedGenerator;
    match check {
        Check::SecondsToNanos => {
            assert_eq!(
                to_unix_nanos(Some(Timestamp::Seconds(Decimal::new(12_345, 1)))),
                Some(1_234_500_000_000)
            );
        }
        Check::SecondsTruncateTowardZero => {
            assert_eq!(
                to_unix_nanos(Some(Timestamp::Seconds(Decimal::new(-15, 1)))),
                Some(-1_500_000_000)
            );
            assert_eq!(
                to_unix_nanos(Some(Timestamp::Seconds(
                    Decimal::from_str_exact("0.0000000015").unwrap()
                ))),
                Some(1)
            );
        }
        Check::DatetimeToNanos => {
            let datetime = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            assert_eq!(
                to_unix_nanos(Some(Timestamp::DateTime(datetime))),
                Some(1_767_225_600_000_000_000)
            );
        }
        Check::AbsentTimeStaysAbsent => assert_eq!(to_unix_nanos(None), None),
        Check::UuidPassesThrough => assert_eq!(
            resolve_trace_id(Some("123e4567-e89b-12d3-a456-426614174000"), &generator).to_string(),
            "123e4567e89b12d3a456426614174000"
        ),
        Check::UppercaseUuidNormalizes => assert_eq!(
            resolve_trace_id(Some("123E4567-E89B-12D3-A456-426614174000"), &generator).to_string(),
            "123e4567e89b12d3a456426614174000"
        ),
        Check::OtherTextHashes => assert_eq!(
            resolve_trace_id(Some("hello"), &generator).to_string(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e"
        ),
        Check::EmptyTraceIdIsFresh => {
            let fresh = resolve_trace_id(Some(""), &generator);
            assert_eq!(fresh, generator.new_trace_id());
        }
        Check::AbsentTraceIdIsFresh => {
            let fresh = resolve_trace_id(None, &generator);
            assert_eq!(fresh, generator.new_trace_id());
        }
        Check::AbsentObservationIdStaysAbsent => {
            assert_eq!(resolve_observation_id(None), None);
            assert_eq!(resolve_observation_id(Some("")), None);
        }
        Check::CanonicalObservationIdPassesThrough => assert_eq!(
            resolve_observation_id(Some("0102030405060708")),
            Some(SpanId::from_hex("0102030405060708").unwrap())
        ),
        Check::UppercaseObservationIdNormalizes => assert_eq!(
            resolve_observation_id(Some("AABBCCDDEEFF0011")),
            Some(SpanId::from_hex("aabbccddeeff0011").unwrap())
        ),
        Check::OtherTextHashesToObservationId => assert_eq!(
            resolve_observation_id(Some("cd")).map(|id| id.to_string()),
            Some("21e721c35a5823fd".to_owned())
        ),
        Check::AllZeroObservationIdHashes => assert_eq!(
            resolve_observation_id(Some("0000000000000000")).map(|id| id.to_string()),
            Some("fcdb4b423f4e5283".to_owned())
        ),
        Check::TraceIdLengthHashesAsObservationId => assert_eq!(
            resolve_observation_id(Some("123e4567e89b12d3a456426614174000"))
                .map(|id| id.to_string()),
            Some("3c168bcc0c43707f".to_owned())
        ),
    }
}
