use std::time::SystemTime;

use chrono::{DateTime, Utc};
use opentelemetry::trace::Status;

use super::fixtures::{database_env, payload};
use super::support::{Rig, START, instant};
use crate::otel::legacy_lifecycle::context::instant_from_ns;
use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::model::payloads::service::ServiceType;
use crate::otel::service::hooks::{ServiceSpanCall, async_service_failure_hook};

#[rstest::rstest]
#[case::fractions_truncate(
    SpanTime::Seconds(1_704_067_200.9),
    SpanTime::Seconds(1_704_067_202.7),
    instant(1_704_067_200.0),
    instant(1_704_067_202.0)
)]
#[case::whole_seconds_stay(
    SpanTime::Seconds(1_704_067_200.0),
    SpanTime::Seconds(1_704_067_202.0),
    instant(1_704_067_200.0),
    instant(1_704_067_202.0)
)]
// _to_ns reads a datetime through its float seconds, so the nanosecond field can round
#[case::datetimes_scale_through_float_seconds(
    SpanTime::DateTime(DateTime::<Utc>::from_timestamp(1_704_067_200, 500_000_000).expect("timestamp")),
    SpanTime::DateTime(DateTime::<Utc>::from_timestamp(1_704_067_202, 250_000_000).expect("timestamp")),
    instant_from_ns(1_704_067_200_500_000_000),
    instant_from_ns(1_704_067_202_249_999_872)
)]
fn otel_legacy_service_callbacks_behavior_tests_behavior(
    #[case] start: SpanTime,
    #[case] end: SpanTime,
    #[case] expected_start: SystemTime,
    #[case] expected_end: SystemTime,
) {
    let rig = Rig::new();
    let lifecycle = rig.lifecycle();
    let parent = rig.parent(&lifecycle, "proxy", START);
    let payload = payload(ServiceType::Postgres, "query", None, None);
    let call = ServiceSpanCall {
        payload: &payload,
        parent: Some(&parent),
        start_time: start,
        end_time: end,
        event_metadata: None,
    };
    async_service_failure_hook(&lifecycle, &call, Some("boom"), &database_env());
    let span = rig.span("postgres");
    assert_eq!(span.start_time, expected_start);
    assert_eq!(span.end_time, expected_end);
    assert_eq!(span.status, Status::error(""));
}
