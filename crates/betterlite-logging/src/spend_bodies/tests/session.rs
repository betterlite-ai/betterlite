use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone};

use super::support::value;
use crate::spend_bodies::session::{
    SessionInput, SpendTimestamp, batch_trace_session_id, ensure_datetime_utc,
    omits_session_id_when_missing, request_duration_ms, session_id,
};

const INJECTED: &str = "11111111-2222-3333-4444-555555555555";

fn session(
    metadata: Option<&Json>,
    payload: Option<&Json>,
    kwargs: Option<&Json>,
    omit: bool,
) -> Option<String> {
    session_id(&SessionInput {
        kwargs,
        metadata,
        standard_logging_payload: payload,
        omit_when_missing: omit,
        batch_trace_session_id: None,
        injected_uuid: INJECTED,
    })
}

#[rstest::rstest]
#[case::true_stamp(r#"{"litellm_session_id_omitted": true}"#)]
#[case::text_stamp(r#"{"litellm_session_id_omitted": "yes"}"#)]
#[case::number_stamp(r#"{"litellm_session_id_omitted": 1}"#)]
fn reads_a_present_omission_stamp(#[case] metadata: &str) {
    assert!(omits_session_id_when_missing(Some(&value(metadata)), false));
}

#[rstest::rstest]
#[case::absent("{}")]
#[case::false_stamp(r#"{"litellm_session_id_omitted": false}"#)]
#[case::empty_text(r#"{"litellm_session_id_omitted": ""}"#)]
#[case::zero(r#"{"litellm_session_id_omitted": 0}"#)]
#[case::null(r#"{"litellm_session_id_omitted": null}"#)]
fn ignores_an_absent_or_empty_omission_stamp(#[case] metadata: &str) {
    assert!(!omits_session_id_when_missing(
        Some(&value(metadata)),
        false
    ));
}

#[test]
fn the_configured_policy_omits_even_without_a_stamp() {
    assert!(omits_session_id_when_missing(None, true));
}

#[rstest::rstest]
#[case::text(r#"{"session_id": "abc"}"#, Some("abc"))]
#[case::number(r#"{"session_id": 12}"#, Some("12"))]
#[case::empty(r#"{"session_id": ""}"#, None)]
#[case::zero(r#"{"session_id": 0}"#, None)]
#[case::false_value(r#"{"session_id": false}"#, None)]
#[case::missing("{}", None)]
fn under_omission_only_a_present_metadata_session_counts(
    #[case] metadata: &str,
    #[case] expected: Option<&str>,
) {
    let decoded = value(metadata);
    assert_eq!(
        session(Some(&decoded), None, None, true),
        expected.map(str::to_owned)
    );
}

#[test]
fn a_batch_id_outranks_trace_ids() {
    assert_eq!(
        batch_trace_session_id(Some("create_batch"), Some("batch-1_batch_cost")),
        Some("batch-1".to_owned())
    );
    assert_eq!(
        batch_trace_session_id(
            Some("aretrieve_batch"),
            Some("batch-1_batch_cost_batch_cost")
        ),
        Some("batch-1_batch_cost".to_owned())
    );
    assert_eq!(
        batch_trace_session_id(Some("completion"), Some("batch-1")),
        None
    );
    assert_eq!(batch_trace_session_id(Some("create_batch"), Some("")), None);
    assert_eq!(batch_trace_session_id(Some("create_batch"), None), None);
}

#[test]
fn a_batch_session_wins_over_the_standard_trace() {
    let payload = value(r#"{"trace_id": "trace"}"#);
    let kwargs = value(r#"{"litellm_trace_id": "other"}"#);
    let session = session_id(&SessionInput {
        kwargs: Some(&kwargs),
        metadata: None,
        standard_logging_payload: Some(&payload),
        omit_when_missing: false,
        batch_trace_session_id: Some("batch"),
        injected_uuid: INJECTED,
    });
    assert_eq!(session, Some("batch".to_owned()));
}

#[test]
fn an_empty_or_null_trace_id_follows_the_source_conversion() {
    let empty_trace = value(r#"{"trace_id": ""}"#);
    assert_eq!(
        session(None, Some(&empty_trace), None, false),
        Some(String::new())
    );
    let numeric_trace = value(r#"{"trace_id": 7}"#);
    assert_eq!(
        session(None, Some(&numeric_trace), None, false),
        Some("7".to_owned())
    );
    let boolean_trace = value(r#"{"trace_id": false}"#);
    assert_eq!(
        session(None, Some(&boolean_trace), None, false),
        Some("False".to_owned())
    );
    let null_trace = value(r#"{"trace_id": null}"#);
    let kwargs = value(r#"{"litellm_trace_id": "from-kwargs"}"#);
    assert_eq!(
        session(None, Some(&null_trace), Some(&kwargs), false),
        Some("from-kwargs".to_owned())
    );
}

#[test]
fn falls_back_to_the_injected_uuid() {
    assert_eq!(session(None, None, None, false), Some(INJECTED.to_owned()));
    let kwargs = value(r#"{"litellm_trace_id": null}"#);
    assert_eq!(
        session(None, None, Some(&kwargs), false),
        Some(INJECTED.to_owned())
    );
}

fn aware(text: &str) -> SpendTimestamp {
    SpendTimestamp::Aware(DateTime::parse_from_rfc3339(text).expect("aware timestamp"))
}

fn naive() -> SpendTimestamp {
    SpendTimestamp::Naive(
        NaiveDate::from_ymd_opt(2024, 1, 2)
            .expect("date")
            .and_hms_micro_opt(3, 4, 5, 600_000)
            .expect("time"),
    )
}

#[test]
fn durations_truncate_toward_zero_including_negative_values() {
    assert_eq!(
        request_duration_ms(
            aware("2024-01-02T03:04:05+00:00"),
            aware("2024-01-02T03:04:06.500+00:00")
        ),
        Some(1500)
    );
    assert_eq!(
        request_duration_ms(
            aware("2024-01-02T03:04:06.500+00:00"),
            aware("2024-01-02T03:04:05+00:00")
        ),
        Some(-1500)
    );
    assert_eq!(
        request_duration_ms(
            aware("2024-01-02T03:04:05+00:00"),
            aware("2024-01-02T03:04:05.023+00:00")
        ),
        Some(23)
    );
    assert_eq!(request_duration_ms(naive(), naive()), Some(0));
}

#[test]
fn a_failed_subtraction_returns_none() {
    assert_eq!(
        request_duration_ms(aware("2024-01-02T03:04:05+00:00"), naive()),
        None
    );
}

#[test]
fn converts_aware_and_naive_timestamps_to_utc() {
    let aware_utc = ensure_datetime_utc(aware("2024-01-02T03:04:05.600+02:00"), &chrono::Utc)
        .expect("aware conversion");
    assert_eq!(aware_utc.to_rfc3339(), "2024-01-02T01:04:05.600+00:00");
    let west = FixedOffset::west_opt(18_000).expect("offset");
    let naive_utc = ensure_datetime_utc(naive(), &west).expect("naive conversion");
    assert_eq!(naive_utc.to_rfc3339(), "2024-01-02T08:04:05.600+00:00");
}

#[test]
fn a_naive_timestamp_is_read_in_the_given_local_zone() {
    let east = FixedOffset::east_opt(10_800).expect("offset");
    let converted = ensure_datetime_utc(naive(), &east).expect("naive conversion");
    assert_eq!(converted.to_rfc3339(), "2024-01-02T00:04:05.600+00:00");
}

#[derive(Clone)]
struct GapZone;

impl TimeZone for GapZone {
    type Offset = FixedOffset;

    fn from_offset(_offset: &FixedOffset) -> Self {
        Self
    }

    fn offset_from_local_date(&self, _local: &NaiveDate) -> LocalResult<FixedOffset> {
        LocalResult::None
    }

    fn offset_from_local_datetime(&self, _local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        LocalResult::None
    }

    fn offset_from_utc_date(&self, _utc: &NaiveDate) -> FixedOffset {
        FixedOffset::east_opt(0).expect("offset")
    }

    fn offset_from_utc_datetime(&self, _utc: &NaiveDateTime) -> FixedOffset {
        FixedOffset::east_opt(0).expect("offset")
    }
}

#[test]
fn a_naive_timestamp_inside_a_local_gap_has_no_utc_instant() {
    assert!(ensure_datetime_utc(naive(), &GapZone).is_none());
}
