#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::execution::Sample;
use betterlite_lens::models::finding::{Evidence, Finding, FindingDraft, Kind, Priority, Role};
use betterlite_lens::models::job::Lens;

use crate::lens_access::tests::support::finding;
use crate::lens_worker_routes::error::LensWorkerError;
use crate::lens_worker_routes::evidence::validate_finding;
use crate::lens_worker_routes::tests::support::{
    TestError, WorkerStorage, base_lens, sample_of, trace_execution,
};

const EXISTING_CHECK: &str = "Existing finding must belong to the same check";
const EVIDENCE_MISMATCH: &str = "Evidence quote does not match stored content";
const MISSING_RUN: &str = "the evidence names a run outside this job";

fn evidence(execution_id: &str) -> Evidence {
    Evidence {
        execution_id: execution_id.to_owned(),
        span_id: "s1".to_owned(),
        quote: "boom".to_owned(),
        role: Role::Support,
    }
}

fn finding_draft(check_id: &str, existing: Option<&str>, runs: &[&str]) -> FindingDraft {
    FindingDraft {
        title: "Checkout retries".to_owned(),
        description: "The checkout retried twice".to_owned(),
        check_id: check_id.to_owned(),
        kind: Kind::Issue,
        priority: Priority::Medium,
        suggestion: String::new(),
        limitation: String::new(),
        evidence: runs.iter().map(|run| evidence(run)).collect(),
        existing_finding_id: existing.map(str::to_owned),
    }
}

fn selected() -> Sample {
    sample_of(vec![trace_execution(0)])
}

fn lens_with(findings: Vec<Finding>) -> Lens {
    let mut stored = base_lens("l1");
    stored.findings = findings;
    stored
}

fn assert_refusal(error: &LensWorkerError<TestError>, status: StatusCode, detail: &str) {
    assert_eq!(error.status(), status);
    assert_eq!(error.detail(), detail);
}

#[tokio::test]
async fn an_unnamed_draft_needs_no_previous_finding() {
    let lens = lens_with(Vec::new());
    let draft = finding_draft("c1", None, &[]);
    validate_finding(&lens, &selected(), &draft, None::<&WorkerStorage>)
        .await
        .expect("a draft without a previous finding is valid");
}

#[tokio::test]
async fn a_named_finding_of_the_same_check_passes() {
    let lens = lens_with(vec![finding("f1")]);
    let draft = finding_draft("c1", Some("f1"), &[]);
    validate_finding(&lens, &selected(), &draft, None::<&WorkerStorage>)
        .await
        .expect("the named finding is the same check");
}

#[tokio::test]
async fn an_unknown_previous_finding_is_422() {
    let lens = lens_with(Vec::new());
    let draft = finding_draft("c1", Some("missing"), &[]);
    let error = validate_finding(&lens, &selected(), &draft, None::<&WorkerStorage>)
        .await
        .expect_err("the named finding does not exist");
    assert_refusal(&error, StatusCode::UNPROCESSABLE_ENTITY, EXISTING_CHECK);
}

#[tokio::test]
async fn a_previous_finding_of_another_check_is_422() {
    let lens = lens_with(vec![finding("f1")]);
    let draft = finding_draft("c2", Some("f1"), &[]);
    let error = validate_finding(&lens, &selected(), &draft, None::<&WorkerStorage>)
        .await
        .expect_err("the named finding belongs to another check");
    assert_refusal(&error, StatusCode::UNPROCESSABLE_ENTITY, EXISTING_CHECK);
}

#[tokio::test]
async fn the_same_check_rule_runs_before_storage() {
    let lens = lens_with(Vec::new());
    let draft = finding_draft("c1", Some("missing"), &["t0"]);
    let error = validate_finding(&lens, &selected(), &draft, None::<&WorkerStorage>)
        .await
        .expect_err("the same-check rule fails first");
    assert_refusal(&error, StatusCode::UNPROCESSABLE_ENTITY, EXISTING_CHECK);
}

#[tokio::test]
async fn matching_evidence_passes() {
    let storage = WorkerStorage::with_pages(&[]);
    let lens = lens_with(Vec::new());
    let execution = trace_execution(0);
    let sample = sample_of(vec![execution.clone()]);
    let draft = finding_draft("c1", None, &[execution.id.as_str()]);
    validate_finding(&lens, &sample, &draft, Some(&storage))
        .await
        .expect("the stored span carries the quote");
    let calls = storage.evidence_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, execution.trace_id);
    assert_eq!(calls[0].span, "s1");
    assert_eq!(calls[0].quote, "boom");
}

#[tokio::test]
async fn mismatching_evidence_is_422() {
    let storage = WorkerStorage::with_pages(&[]).answering_evidence(false);
    let lens = lens_with(Vec::new());
    let execution = trace_execution(0);
    let sample = sample_of(vec![execution.clone()]);
    let draft = finding_draft("c1", None, &[execution.id.as_str()]);
    let error = validate_finding(&lens, &sample, &draft, Some(&storage))
        .await
        .expect_err("the quote is not in the stored span");
    assert_refusal(&error, StatusCode::UNPROCESSABLE_ENTITY, EVIDENCE_MISMATCH);
}

#[tokio::test]
async fn evidence_without_storage_is_501() {
    let lens = lens_with(Vec::new());
    let draft = finding_draft("c1", None, &["t0"]);
    let error = validate_finding(&lens, &selected(), &draft, None::<&WorkerStorage>)
        .await
        .expect_err("there is no tracing store to verify against");
    assert_eq!(error.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn an_evidence_run_outside_the_sample_is_500() {
    let storage = WorkerStorage::with_pages(&[]);
    let lens = lens_with(Vec::new());
    let draft = finding_draft("c1", None, &["missing"]);
    let error = validate_finding(&lens, &selected(), &draft, Some(&storage))
        .await
        .expect_err("the run is not in the sample");
    assert_refusal(&error, StatusCode::INTERNAL_SERVER_ERROR, MISSING_RUN);
    assert!(storage.evidence_calls().is_empty());
}

#[tokio::test]
async fn a_storage_failure_is_500() {
    let storage = WorkerStorage::with_pages(&[]).failing();
    let lens = lens_with(Vec::new());
    let execution = trace_execution(0);
    let sample = sample_of(vec![execution.clone()]);
    let draft = finding_draft("c1", None, &[execution.id.as_str()]);
    let error = validate_finding(&lens, &sample, &draft, Some(&storage))
        .await
        .expect_err("the fake storage failed");
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.detail().contains("the fake storage failed"));
}

#[tokio::test]
async fn every_evidence_entry_is_checked_in_order() {
    let storage = WorkerStorage::with_pages(&[]);
    let lens = lens_with(Vec::new());
    let first = trace_execution(0);
    let second = trace_execution(1);
    let sample = sample_of(vec![first.clone(), second.clone()]);
    let draft = finding_draft("c1", None, &[first.id.as_str(), "missing"]);
    let error = validate_finding(&lens, &sample, &draft, Some(&storage))
        .await
        .expect_err("the second entry names no run");
    assert_refusal(&error, StatusCode::INTERNAL_SERVER_ERROR, MISSING_RUN);
    assert_eq!(storage.evidence_calls().len(), 1);
}
