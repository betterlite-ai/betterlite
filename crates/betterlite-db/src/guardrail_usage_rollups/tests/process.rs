use chrono::DateTime;
use rust_decimal::Decimal;

use crate::guardrail_usage_rollups::{
    GuardrailCounts, GuardrailIndexRow, GuardrailSpendLog, MetricsKey, index_rows,
    process_guardrail_usage,
};
use crate::spend_batches::StatementLimits;

use super::support::{
    FakeSink, NOT_RETRY_SAFE, PauseLog, RETRY_SAFE, UnitCall, capture_logs, from_json, lock, shared,
};

const ONE_ENTRY: &str = r#"{
    "request_id": "req-1",
    "startTime": "2026-01-02T03:04:05Z",
    "team_id": "team-1",
    "api_key": "sk-1",
    "metadata": {"guardrail_information": [
        {"guardrail_id": "g1", "guardrail_name": "pii", "guardrail_status": "success",
         "guardrail_usage": {"contentPolicyUnits": 2},
         "guardrail_cost_by_unit": {"contentPolicyUnits": 0.5}}
    ]}
}"#;

const METRICS_ONLY: &str = r#"{
    "request_id": "req-1",
    "startTime": "2026-01-02T03:04:05Z",
    "metadata": {"guardrail_information": [{"guardrail_id": "g1", "guardrail_status": "success"}]}
}"#;

const fn limits(max_bytes: usize, max_rows: usize) -> StatementLimits {
    StatementLimits {
        max_bytes,
        max_rows,
    }
}

fn metrics_key() -> MetricsKey {
    MetricsKey {
        guardrail_id: String::from("g1"),
        date: String::from("2026-01-02"),
    }
}

fn counts(requests: i64, passed: i64) -> GuardrailCounts {
    GuardrailCounts {
        requests_evaluated: requests,
        passed_count: passed,
        blocked_count: 0,
        flagged_count: 0,
    }
}

fn payload_for(request_id: &str) -> GuardrailSpendLog {
    from_json(&format!(
        r#"{{"request_id": "{request_id}", "startTime": "2026-01-02T03:04:05Z", "metadata": {{"guardrail_information": [{{"guardrail_id": "g1"}}]}}}}"#
    ))
}

async fn process(payloads: &[GuardrailSpendLog], sink: &FakeSink) {
    let pending = shared();
    let pause = PauseLog::default();
    process_guardrail_usage(payloads, &pending, sink, limits(1_000_000, 100), &pause).await;
}

#[tokio::test]
async fn writes_the_metrics_units_and_index_row_of_a_payload() {
    let sink = FakeSink::default();
    let pending = shared();
    let pause = PauseLog::default();
    let payload = from_json(ONE_ENTRY);
    process_guardrail_usage(&[payload], &pending, &sink, limits(1_000_000, 100), &pause).await;
    let instant = DateTime::from_timestamp(1_767_323_045, 0).expect("a test instant");
    assert_eq!(
        sink.index_calls(),
        [vec![GuardrailIndexRow::new(
            String::from("req-1"),
            String::from("g1"),
            None,
            instant
        )]]
    );
    assert_eq!(sink.metric_calls(), [(metrics_key(), counts(1, 1))]);
    assert_eq!(
        sink.unit_calls(),
        [UnitCall {
            guardrail_id: String::from("g1"),
            date: String::from("2026-01-02"),
            team_id: String::from("team-1"),
            api_key: String::from("sk-1"),
            usage_unit: String::from("contentPolicyUnits"),
            units: 2,
            cost: Decimal::new(5, 1),
            untracked_units: 0,
        }]
    );
    assert!(lock(&pending).metrics.is_empty());
    assert!(lock(&pending).units.is_empty());
}

#[tokio::test]
async fn keeps_writing_the_rollups_when_an_index_batch_fails() {
    let capture = capture_logs();
    let sink = FakeSink::failing_index(vec![NOT_RETRY_SAFE]);
    let payload = from_json(ONE_ENTRY);
    process(&[payload], &sink).await;
    assert_eq!(sink.index_calls().len(), 1);
    assert_eq!(sink.metric_calls(), [(metrics_key(), counts(1, 1))]);
    assert_eq!(sink.unit_calls().len(), 1);
    assert!(capture.logs.has("DEBUG", "index create_many skipped"));
}

#[tokio::test]
async fn splits_index_rows_into_bounded_batches() {
    let sink = FakeSink::default();
    let pending = shared();
    let pause = PauseLog::default();
    let payloads = [
        payload_for("req-1"),
        payload_for("req-2"),
        payload_for("req-3"),
    ];
    process_guardrail_usage(&payloads, &pending, &sink, limits(1_000_000, 2), &pause).await;
    let lengths: Vec<usize> = sink.index_calls().iter().map(Vec::len).collect();
    assert_eq!(lengths, [2, 1]);
}

#[tokio::test]
async fn requeues_a_retry_safe_upsert_failure_for_the_next_flush() {
    let pending = shared();
    let pause = PauseLog::default();
    let payload = from_json(METRICS_ONLY);
    let failing = FakeSink::failing_upserts(vec![Some(RETRY_SAFE); 4]);
    process_guardrail_usage(
        std::slice::from_ref(&payload),
        &pending,
        &failing,
        limits(1_000_000, 100),
        &pause,
    )
    .await;
    assert_eq!(failing.metric_calls().len(), 4);
    assert_eq!(
        lock(&pending).metrics.get(&metrics_key()),
        Some(&counts(1, 1))
    );
    let healthy = FakeSink::default();
    process_guardrail_usage(
        &[payload],
        &pending,
        &healthy,
        limits(1_000_000, 100),
        &pause,
    )
    .await;
    assert_eq!(healthy.metric_calls(), [(metrics_key(), counts(2, 2))]);
    assert!(lock(&pending).metrics.is_empty());
}

#[tokio::test]
async fn drops_a_rollup_row_that_fails_without_a_retry_safe_error() {
    let capture = capture_logs();
    let pending = shared();
    let pause = PauseLog::default();
    let sink = FakeSink::failing_upserts(vec![Some(NOT_RETRY_SAFE)]);
    let payload = from_json(METRICS_ONLY);
    process_guardrail_usage(&[payload], &pending, &sink, limits(1_000_000, 100), &pause).await;
    assert!(lock(&pending).metrics.is_empty());
    assert!(capture.logs.has("WARN", "not safe to retry"));
}

#[tokio::test]
async fn writes_only_the_index_row_of_a_not_run_entry() {
    let sink = FakeSink::default();
    let payload = from_json(
        r#"{"request_id": "req-1", "startTime": "2026-01-02T03:04:05Z", "metadata": {"guardrail_information": [{"guardrail_id": "g1", "guardrail_status": "not_run"}]}}"#,
    );
    process(&[payload], &sink).await;
    assert_eq!(sink.index_calls().len(), 1);
    assert!(sink.metric_calls().is_empty());
    assert!(sink.unit_calls().is_empty());
}

#[tokio::test]
async fn flushes_every_merged_row_past_the_pending_cap() {
    let sink = FakeSink::default();
    let pending = shared();
    lock(&pending).metrics = (0_u32..10_000)
        .map(|index| {
            (
                MetricsKey {
                    guardrail_id: format!("q{index}"),
                    date: String::from("2026-01-02"),
                },
                counts(1, 1),
            )
        })
        .collect();
    process(&[from_json(METRICS_ONLY)], &sink).await;
    assert_eq!(sink.metric_calls().len(), 10_001);
    assert!(lock(&pending).metrics.is_empty());
}

#[tokio::test]
async fn writes_nothing_when_no_payload_has_guardrail_information() {
    let sink = FakeSink::default();
    let payload = from_json(r#"{"request_id": "req-1", "startTime": "2026-01-02T03:04:05Z"}"#);
    process(&[payload], &sink).await;
    assert!(sink.index_calls().is_empty());
    assert!(sink.metric_calls().is_empty());
    assert!(sink.unit_calls().is_empty());
}

#[tokio::test]
async fn keeps_the_pending_queue_untouched_for_an_empty_batch() {
    let sink = FakeSink::default();
    let pending = shared();
    lock(&pending).metrics.insert(metrics_key(), counts(3, 3));
    let pause = PauseLog::default();
    process_guardrail_usage(&[], &pending, &sink, limits(1_000_000, 100), &pause).await;
    assert!(sink.metric_calls().is_empty());
    assert_eq!(
        lock(&pending).metrics.get(&metrics_key()),
        Some(&counts(3, 3))
    );
}

#[test]
fn keeps_the_first_policy_id_of_a_request_and_guardrail() {
    let payload = from_json(
        r#"{"request_id": "req-1", "startTime": "2026-01-02T03:04:05Z", "metadata": {"guardrail_information": [
            {"guardrail_id": "g1", "policy_id": null},
            {"guardrail_id": "g1", "policy_id": "p1"},
            {"guardrail_id": "g1", "policy_id": "p2"}
        ]}}"#,
    );
    let rows = index_rows::build(&[payload]);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows.first().and_then(|row| row.policy_id.as_deref()),
        Some("p1")
    );
}

#[test]
fn deduplicates_index_rows_for_a_repeated_request_and_guardrail() {
    let rows = index_rows::build(&[payload_for("req-1"), payload_for("req-1")]);
    assert_eq!(rows.len(), 1);
}

#[test]
fn writes_pythons_datetime_text_and_hides_the_instant_from_the_index_json() {
    let whole = GuardrailIndexRow::new(
        String::from("req-1"),
        String::from("g1"),
        None,
        DateTime::from_timestamp(1_767_323_045, 0).expect("a test instant"),
    );
    assert_eq!(whole.start_time, "2026-01-02 03:04:05+00:00");
    assert_eq!(
        serde_json::to_string(&whole).expect("the row serializes"),
        r#"{"request_id":"req-1","guardrail_id":"g1","policy_id":null,"start_time":"2026-01-02 03:04:05+00:00"}"#
    );
    let fractional = GuardrailIndexRow::new(
        String::from("req-1"),
        String::from("g1"),
        None,
        DateTime::from_timestamp(1_767_323_045, 123_456_000).expect("a test instant"),
    );
    assert_eq!(fractional.start_time, "2026-01-02 03:04:05.123456+00:00");
}
