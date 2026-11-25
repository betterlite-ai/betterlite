#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::job::{Job, JobStatus, Lens};
use betterlite_lens::models::settings::LensSettings;

use crate::lens_access::tests::support::{
    Harness, admin, body_json, call, finding, job, lens, member, minutes_after, now, request,
    settings, settings_json,
};

const WRITE_DENIED: &str = "Only proxy admins can configure or run Lens";
const UNKNOWN_MODEL: &str = "Choose a model configured on this LiteLLM instance";

#[tokio::test]
async fn run_queues_a_job() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/runs", Some(&admin()), Some("{}")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    let [only] = updated.jobs.as_slice() else {
        panic!("the run queues one job");
    };
    assert_eq!(only.id, "id-1");
    assert_eq!(only.status, JobStatus::Queued);
    assert_eq!(only.created_at, now());
    assert_eq!(only.start, minutes_after(now(), -1440));
    assert_eq!(only.end, minutes_after(now(), -2));
    assert_eq!(only.revision, 1);
    assert_eq!(harness.database.lens("l1").jobs.len(), 1);
}

#[tokio::test]
async fn a_run_request_overrides_settings_and_lookback() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let request_body = format!(
        r#"{{"settings":{},"lookback_hours":2}}"#,
        settings_json("gpt-4o")
    );
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/runs", Some(&admin()), Some(&request_body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    let [only] = updated.jobs.as_slice() else {
        panic!("the run queues one job");
    };
    assert_eq!(only.start, minutes_after(now(), -120));
}

#[tokio::test]
async fn a_run_with_settings_checks_the_model() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let request_body = body_json(&RunBody {
        settings: Some(settings("not-deployed")),
        lookback_hours: None,
    });
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/runs", Some(&admin()), Some(&request_body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail(), UNKNOWN_MODEL);
}

#[derive(serde::Serialize)]
struct RunBody {
    settings: Option<LensSettings>,
    lookback_hours: Option<u32>,
}

#[tokio::test]
async fn a_second_run_keeps_the_existing_job() {
    let harness = Harness::new();
    let mut seeded = lens("l1");
    seeded.jobs = vec![job("old", JobStatus::Queued)];
    harness.database.seed_lens(seeded);
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/runs", Some(&admin()), Some("{}")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    let [only] = updated.jobs.as_slice() else {
        panic!("the queued job stays");
    };
    assert_eq!(only.id, "old");
    let queries = harness.database.queries();
    assert!(!queries.iter().any(|query| query.contains("WITH previous")));
}

#[tokio::test]
async fn run_denies_a_non_admin_writer() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/runs", Some(&member()), Some("{}")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_DENIED);
}

#[tokio::test]
async fn list_runs_strips_sample_findings_and_assessments() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let mut archived = job("j1", JobStatus::Completed);
    archived.findings = Some(vec![finding("f1")]);
    archived.assessments =
        vec![serde_json::from_str(r#"{"execution_id":"e1"}"#).expect("the assessment fixture")];
    archived.sample = Some(
        serde_json::from_str(r#"{"executions":[],"eligible":0}"#).expect("the sample fixture"),
    );
    harness.database.seed_job(archived);
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1/runs", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let jobs: Vec<Job> = reply.json();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, "j1");
    assert!(jobs[0].sample.is_none());
    assert!(jobs[0].findings.is_none());
    assert!(jobs[0].assessments.is_empty());
}

#[tokio::test]
async fn list_runs_passes_the_offset() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1/runs?offset=7", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let zero = call(
        harness.app(),
        request("GET", "/lens/l1/runs?offset=0", Some(&admin()), None),
    )
    .await;
    assert_eq!(zero.status, StatusCode::OK);
    assert_eq!(harness.database.job_offsets(), [7, 0]);
}

#[tokio::test]
async fn list_runs_rejects_a_non_integer_offset() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1/runs?offset=abc", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "int_parsing");
    assert_eq!(issues[0].loc, ["query", "offset"]);
}

#[tokio::test]
async fn list_runs_rejects_a_negative_offset() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1/runs?offset=-1", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "greater_than_equal");
}

#[tokio::test]
async fn read_run_returns_the_full_job() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let mut archived = job("j1", JobStatus::Completed);
    archived.findings = Some(vec![finding("f1")]);
    harness.database.seed_job(archived);
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1/runs/j1", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let read: Job = reply.json();
    assert_eq!(read.id, "j1");
    assert!(read.findings.is_some());
}

#[tokio::test]
async fn read_run_missing_is_404() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1/runs/missing", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Investigation not found");
}

#[tokio::test]
async fn read_run_for_a_missing_lens_is_not_found() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request("GET", "/lens/missing/runs/j1", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn cancel_moves_the_next_run_and_finishes_the_job() {
    let harness = Harness::new();
    let mut seeded = lens("l1");
    seeded.jobs = vec![job("j1", JobStatus::Queued)];
    harness.database.seed_lens(seeded);
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/cancel", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    assert_eq!(updated.next_run_at, minutes_after(now(), 15));
    let [only] = updated.jobs.as_slice() else {
        panic!("the job stays");
    };
    assert_eq!(only.status, JobStatus::Cancelled);
    assert_eq!(only.stage, "Cancelled");
    assert_eq!(only.finished_at, Some(now()));
    assert_eq!(
        harness.database.lens("l1").next_run_at,
        minutes_after(now(), 15)
    );
}

#[tokio::test]
async fn cancel_without_a_job_keeps_the_lens() {
    let harness = Harness::new();
    let seeded = lens("l1");
    harness.database.seed_lens(seeded);
    let reply = call(
        harness.app(),
        request("POST", "/lens/l1/cancel", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let unchanged: Lens = reply.json();
    assert_eq!(
        unchanged.next_run_at.to_rfc3339(),
        "2026-01-01T00:00:00+00:00"
    );
    let queries = harness.database.queries();
    assert!(!queries.iter().any(|query| query.contains("WITH previous")));
}
