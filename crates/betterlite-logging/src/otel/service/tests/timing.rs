use super::support::NOW;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::service::timing::whole_second_ns;

#[rstest::rstest]
#[case::fractional_seconds_truncate(SpanTime::Seconds(1_704_067_200.9), 1_704_067_200_000_000_000)]
#[case::negative_fractions_truncate_toward_zero(SpanTime::Seconds(-1.5), -1_000_000_000)]
#[case::whole_seconds_stay(SpanTime::Seconds(1_704_067_202.0), 1_704_067_202_000_000_000)]
#[case::missing_takes_now(SpanTime::Missing, NOW)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] time: SpanTime,
    #[case] expected: i64,
) {
    assert_eq!(whole_second_ns(time, NOW), expected);
}
