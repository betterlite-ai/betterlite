#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::job::JobStatus;
use betterlite_lens::sources::activity_availability::ActivityAvailability;

use crate::lens_access::tests::support::{
    Harness, admin, call, execution_id, finding, job, lens, request, settings_json, worker,
};

fn seeded() -> Harness {
    let harness = Harness::new();
    let mut stored = lens("l1");
    stored.jobs = vec![job("j1", JobStatus::Queued)];
    stored.findings = vec![finding("f1")];
    harness.database.seed_lens(stored);
    harness.database.seed_worker(worker("w1"));
    harness
}

fn key_id() -> String {
    "c".repeat(64)
}

const UPDATE_BODY: &str =
    r#"{"name":"Updated","model":"gpt-4o","context":"Watch the checkout flow"}"#;
const RESOLVED_BODY: &str = r#"{"status":"resolved"}"#;
const PREVIEW_BODY: &str =
    r#"{"settings":{"name":"Nightly","model":"gpt-4o","context":"Watch the checkout flow"}}"#;

async fn status(harness: &Harness, method: &str, uri: &str, body: Option<&str>) -> StatusCode {
    let caller = admin();
    call(harness.app(), request(method, uri, Some(&caller), body))
        .await
        .status
}

#[tokio::test]
async fn every_lens_route_answers() {
    let harness = seeded();
    let settings = settings_json("gpt-4o");
    assert_eq!(status(&harness, "GET", "/lens", None).await, StatusCode::OK);
    assert_eq!(
        status(&harness, "POST", "/lens", Some(settings.as_str())).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "GET", "/lens/l1", None).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "PUT", "/lens/l1", Some(UPDATE_BODY)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn every_discovery_route_answers() {
    let harness = seeded();
    assert_eq!(
        status(&harness, "GET", "/lens/activity/available", None).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "GET", "/lens/agents", None).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "POST", "/lens/preview/sample", Some(PREVIEW_BODY)).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn every_run_route_answers() {
    let harness = seeded();
    assert_eq!(
        status(&harness, "POST", "/lens/l1/runs", Some("{}")).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "GET", "/lens/l1/runs", None).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "GET", "/lens/l1/runs/j1", None).await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "POST", "/lens/l1/cancel", None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn every_worker_route_answers() {
    let harness = seeded();
    let register = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let billing = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    assert_eq!(
        status(
            &harness,
            "POST",
            "/lens/workers/register",
            Some(register.as_str())
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status(
            &harness,
            "PUT",
            "/lens/workers/w1/billing-key",
            Some(billing.as_str())
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "DELETE", "/lens/workers/w1", None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn every_finding_and_evidence_route_answers() {
    let harness = seeded();
    let trace = execution_id(&["traces", "team-1", "trace-1"]);
    let evidence = format!("/lens/l1/executions/{trace}");
    assert_eq!(
        status(
            &harness,
            "PATCH",
            "/lens/l1/findings/f1",
            Some(RESOLVED_BODY)
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        status(&harness, "GET", evidence.as_str(), None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_static_agents_route_wins_over_a_lens_id() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("agents"));
    let reply = call(
        harness.app(),
        request("GET", "/lens/agents", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let agents: Vec<String> = reply.json();
    assert!(agents.is_empty());
}

#[tokio::test]
async fn the_static_activity_route_answers_availability() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request("GET", "/lens/activity/available", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let available: ActivityAvailability = reply.json();
    assert!(!available.traces);
}

#[tokio::test]
async fn no_worker_route_is_mounted() {
    let harness = Harness::new();
    let claim = call(
        harness.app(),
        request("POST", "/lens/worker/claim", Some(&admin()), Some("{}")),
    )
    .await;
    assert_eq!(claim.status, StatusCode::NOT_FOUND);
    let assigned = call(
        harness.app(),
        request("GET", "/lens/worker/assigned", Some(&admin()), None),
    )
    .await;
    assert_eq!(assigned.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unauthenticated_call_is_401() {
    let harness = Harness::new();
    let reply = call(harness.app(), request("GET", "/lens", None, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let text = String::from_utf8(reply.body).expect("the body is utf-8");
    assert!(text.contains("auth_error"));
}

#[tokio::test]
async fn an_unknown_path_is_404() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request("GET", "/lensx", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}
