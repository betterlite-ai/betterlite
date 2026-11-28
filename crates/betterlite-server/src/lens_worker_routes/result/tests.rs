#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::execution::Execution;
use betterlite_lens::models::finding::FindingStatus;
use betterlite_lens::models::job::{JobStatus, Lens};
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::tests::support::{finding, now};
use crate::lens_worker_routes::tests::support::{
    Harness, WorkerReply, WorkerStorage, assessment, authorized, base_lens, draft, result_body,
    running_job, sample_of, trace_execution, with_checks, worker_call, worker_fixture,
};

const NOT_OWNED: &str = "This worker no longer owns the job";
const NO_DATABASE: &str = "Lens needs a connected Postgres database";
const TRACING_DISABLED: &str =
    "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.";
const DUPLICATE_RUN: &str = "Each run must have one assessment";
const RUN_OUTSIDE: &str = "Assessment references a run outside this job";
const CHECK_UNKNOWN: &str = "Assessment references an unknown check";
const FINDING_OUTSIDE: &str = "Finding references evidence outside the job";
const EXISTING_CHECK: &str = "Existing finding must belong to the same check";
const EVIDENCE_MISMATCH: &str = "Evidence quote does not match stored content";

fn later(millis: i64) -> DateTime<Utc> {
    now()
        .checked_add_signed(TimeDelta::milliseconds(millis))
        .expect("a representable instant")
}

fn interval_end() -> DateTime<Utc> {
    now()
        .checked_add_signed(TimeDelta::minutes(15))
        .expect("a representable instant")
}

fn job_end() -> DateTime<Utc> {
    "2026-01-02T00:00:00Z".parse().expect("the fixture job end")
}

fn scanned_at() -> DateTime<Utc> {
    "2026-04-01T00:00:00Z".parse().expect("a scan time")
}

fn sampled_lens(checks: &[&str], indexes: &[usize]) -> Lens {
    let mut stored = with_checks(base_lens("l1"), checks);
    let mut job = running_job("j1", "w1", later(300_000));
    job.settings = stored.settings.clone();
    job.sample = Some(sample_of(
        indexes
            .iter()
            .map(|index| trace_execution(*index))
            .collect(),
    ));
    stored.jobs = vec![job];
    stored
}

fn finished_lens(status: JobStatus) -> Lens {
    let mut stored = with_checks(base_lens("l1"), &["c1"]);
    let mut job = running_job("j1", "w1", later(300_000));
    job.status = status;
    stored.jobs = vec![job];
    stored
}

async fn post_result(harness: &Harness, body: &str) -> WorkerReply {
    worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/l1/j1/result", Some(body)),
    )
    .await
}

fn assert_finished_run(stored: &Lens, first: &Execution) {
    assert_eq!(stored.version, 1);
    let job = &stored.jobs[0];
    assert_eq!(job.status, JobStatus::Completed);
    assert_eq!(job.stage, "Complete");
    assert_eq!(job.finished_at, Some(now()));
    assert_eq!(job.coverage.screened, 5);
    assert_eq!(job.error, "");
    assert_eq!(job.assessments.len(), 1);
    assert_eq!(job.assessments[0].execution_id, first.id);
    assert_eq!(job.assessments[0].issue_checks, ["c1"]);
    let snapshots = job
        .findings
        .as_ref()
        .expect("the job keeps the run's findings");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].check_id, "c1");
    assert_eq!(snapshots[0].title, "Checkout retries");
    assert_eq!(snapshots[0].revision, 1);
    assert_eq!(snapshots[0].status, FindingStatus::Open);
    assert_eq!(snapshots[0].occurrences, [first.id.clone()]);
    assert_eq!(snapshots[0].id, stored.findings[0].id);
    assert_eq!(stored.findings[0].title, "Checkout retries");
    assert_eq!(stored.last_scan_at, Some(job_end()));
    assert_eq!(stored.next_run_at, interval_end());
}

#[tokio::test]
async fn result_finishes_the_job_and_merges_its_findings() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0, 1]));
    let first = trace_execution(0);
    let body = result_body(
        &[assessment(&first.id, &["c1"])],
        &[draft("c1", None, &first.id)],
        "",
    );
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    assert_finished_run(&stored, &first);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .evidence_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].span, "s1");
    assert_eq!(calls[0].quote, "boom");
    assert_eq!(calls[0].id, "t0");
}

#[tokio::test]
async fn older_findings_stay_after_the_merged_one() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut lens = sampled_lens(&["c1"], &[0]);
    lens.findings = vec![finding("f1"), finding("f2")];
    harness.database.seed_lens(lens);
    let first = trace_execution(0);
    let body = result_body(&[], &[draft("c1", None, &first.id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    assert_eq!(stored.findings.len(), 3);
    assert_eq!(stored.findings[1].id, "f1");
    assert_eq!(stored.findings[2].id, "f2");
}

#[tokio::test]
async fn result_verifies_every_finding() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0, 1]));
    let first = trace_execution(0);
    let second = trace_execution(1);
    let body = result_body(
        &[],
        &[draft("c1", None, &first.id), draft("c1", None, &second.id)],
        "",
    );
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .evidence_calls();
    assert_eq!(calls.len(), 2);
    let snapshots = stored.jobs[0]
        .findings
        .as_ref()
        .expect("the job keeps both drafts");
    assert_eq!(snapshots.len(), 2);
    assert_eq!(stored.findings.len(), 1);
}

#[tokio::test]
async fn a_retried_result_returns_the_lens_at_once() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    let stored = finished_lens(JobStatus::Completed);
    harness.database.seed_lens(stored.clone());
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Lens = reply.json();
    assert_eq!(body, stored);
    assert!(harness.database.events().is_empty());
}

#[tokio::test]
async fn a_failed_retry_is_returned_at_once() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    let stored = finished_lens(JobStatus::Failed);
    harness.database.seed_lens(stored.clone());
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Lens = reply.json();
    assert_eq!(body, stored);
}

#[tokio::test]
async fn a_finished_job_of_another_worker_goes_on() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = finished_lens(JobStatus::Completed);
    stored.jobs[0].worker_id = Some("w2".to_owned());
    harness.database.seed_lens(stored);
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_OWNED);
}

#[tokio::test]
async fn a_queued_job_goes_on() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(finished_lens(JobStatus::Queued));
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_OWNED);
}

#[tokio::test]
async fn an_upload_with_an_error_finishes_failed() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut lens = sampled_lens(&["c1"], &[0]);
    lens.jobs[0].coverage.screened = 2;
    lens.last_scan_at = Some(scanned_at());
    harness.database.seed_lens(lens);
    let reply = post_result(&harness, &result_body(&[], &[], "model timed out")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    let job = &stored.jobs[0];
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.stage, "Failed");
    assert_eq!(job.coverage.screened, 2);
    assert_eq!(job.error, "model timed out");
    assert_eq!(job.findings.as_ref().map(Vec::len), Some(0));
    assert_eq!(stored.last_scan_at, Some(scanned_at()));
    assert_eq!(stored.next_run_at, interval_end());
    assert!(stored.findings.is_empty());
}

#[tokio::test]
async fn last_scan_at_keeps_the_later_value() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut lens = sampled_lens(&["c1"], &[0]);
    lens.last_scan_at = Some(scanned_at());
    harness.database.seed_lens(lens);
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    assert_eq!(stored.last_scan_at, Some(scanned_at()));
}

#[tokio::test]
async fn result_without_a_sample_still_finishes() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut lens = with_checks(base_lens("l1"), &["c1"]);
    lens.jobs = vec![running_job("j1", "w1", later(300_000))];
    harness.database.seed_lens(lens);
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    assert_eq!(stored.jobs[0].status, JobStatus::Completed);
    assert_eq!(stored.jobs[0].coverage.screened, 5);
    assert_eq!(stored.jobs[0].findings.as_ref().map(Vec::len), Some(0));
}

#[tokio::test]
async fn a_duplicate_assessment_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let body = result_body(
        &[
            assessment(&first.id, &["c1"]),
            assessment(&first.id, &["c1"]),
        ],
        &[],
        "",
    );
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), DUPLICATE_RUN);
}

#[tokio::test]
async fn an_assessment_outside_the_sample_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let body = result_body(&[assessment(&trace_execution(5).id, &["nope"])], &[], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), RUN_OUTSIDE);
}

#[tokio::test]
async fn an_unknown_assessment_check_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let body = result_body(&[assessment(&first.id, &["nope"])], &[], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), CHECK_UNKNOWN);
}

#[tokio::test]
async fn an_unknown_pattern_check_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let listed = format!(
        r#"{{"execution_id":"{}","pattern_checks":["nope"]}}"#,
        first.id
    );
    let body = result_body(&[listed], &[], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), CHECK_UNKNOWN);
}

#[tokio::test]
async fn an_assessment_may_name_a_pattern_check() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let listed = format!(
        r#"{{"execution_id":"{}","issue_checks":["expected_behavior"],"pattern_checks":["c1"]}}"#,
        first.id
    );
    let body = result_body(&[listed], &[], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored: Lens = reply.json();
    assert_eq!(stored.jobs[0].assessments[0].pattern_checks, ["c1"]);
}

#[tokio::test]
async fn a_finding_with_an_unknown_check_is_422() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let body = result_body(&[], &[draft("nope", None, &trace_execution(0).id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), FINDING_OUTSIDE);
}

#[tokio::test]
async fn a_finding_with_evidence_outside_the_sample_is_422() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let body = result_body(&[], &[draft("c1", None, &trace_execution(5).id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), FINDING_OUTSIDE);
}

#[tokio::test]
async fn an_unknown_previous_finding_is_422_before_storage() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let body = result_body(&[], &[draft("c1", Some("missing"), &first.id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), EXISTING_CHECK);
}

#[tokio::test]
async fn a_previous_finding_of_another_check_is_422() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    let mut lens = sampled_lens(&["c1", "c2"], &[0]);
    lens.findings = vec![finding("f1")];
    harness.database.seed_lens(lens);
    let first = trace_execution(0);
    let body = result_body(&[], &[draft("c2", Some("f1"), &first.id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), EXISTING_CHECK);
}

#[tokio::test]
async fn evidence_that_does_not_match_is_422() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[]).answering_evidence(false));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let body = result_body(&[], &[draft("c1", None, &first.id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), EVIDENCE_MISMATCH);
}

#[tokio::test]
async fn a_finding_without_storage_is_501() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let first = trace_execution(0);
    let body = result_body(&[], &[draft("c1", None, &first.id)], "");
    let reply = post_result(&harness, &body).await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(reply.detail(), TRACING_DISABLED);
}

#[tokio::test]
async fn the_finish_transform_leaves_a_moved_lens_alone() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(sampled_lens(&["c1"], &[0]));
    let mut replacement = with_checks(base_lens("l1"), &["c1"]);
    replacement.jobs = vec![running_job("j1", "w2", later(300_000))];
    harness.database.moved_on_read(3, replacement.clone());
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: Lens = reply.json();
    assert_eq!(body, replacement);
    assert_eq!(harness.database.lens("l1"), replacement);
}

#[tokio::test]
async fn result_without_a_database_is_503_before_the_body() {
    let harness = Harness::new().without_database();
    let reply = post_result(&harness, &result_body(&[], &[], "")).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.detail(), NO_DATABASE);
}

#[tokio::test]
async fn a_result_without_coverage_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = post_result(&harness, r#"{"assessments":[],"findings":[]}"#).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(issues[0].loc, ["body", "coverage"]);
}
