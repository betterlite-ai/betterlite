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

#[tokio::test]
async fn every_listed_route_answers() {
    let harness = seeded();
    let settings = settings_json("gpt-4o");
    let update = r#"{"name":"Updated","model":"gpt-4o"}"#;
    let preview = r#"{"settings":{"name":"Nightly","model":"gpt-4o"}}"#;
    let register = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let billing = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let evidence = execution_id(&["traces", "team-1", "trace-1"]);
    let caller = admin();
    let statuses = [
        call(harness.app(), request("GET", "/lens", Some(&caller), None))
            .await
            .status,
        call(
            harness.app(),
            request("POST", "/lens", Some(&caller), Some(&settings)),
        )
        .await
        .status,
        call(
            harness.app(),
            request("GET", "/lens/activity/available", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request("GET", "/lens/agents", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request("PUT", "/lens/l1", Some(&caller), Some(update)),
        )
        .await
        .status,
        call(
            harness.app(),
            request("POST", "/lens/l1/runs", Some(&caller), Some("{}")),
        )
        .await
        .status,
        call(
            harness.app(),
            request("GET", "/lens/l1", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request("GET", "/lens/l1/runs", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request("GET", "/lens/l1/runs/j1", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request("POST", "/lens/l1/cancel", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request(
                "PATCH",
                "/lens/l1/findings/f1",
                Some(&caller),
                Some(r#"{"status":"resolved"}"#),
            ),
        )
        .await
        .status,
        call(
            harness.app(),
            request("POST", "/lens/preview/sample", Some(&caller), Some(preview)),
        )
        .await
        .status,
        call(
            harness.app(),
            request(
                "POST",
                "/lens/workers/register",
                Some(&caller),
                Some(&register),
            ),
        )
        .await
        .status,
        call(
            harness.app(),
            request(
                "PUT",
                "/lens/workers/w1/billing-key",
                Some(&caller),
                Some(&billing),
            ),
        )
        .await
        .status,
        call(
            harness.app(),
            request("DELETE", "/lens/workers/w1", Some(&caller), None),
        )
        .await
        .status,
        call(
            harness.app(),
            request(
                "GET",
                &format!("/lens/l1/executions/{evidence}"),
                Some(&caller),
                None,
            ),
        )
        .await
        .status,
    ];
    assert_eq!(statuses, [StatusCode::OK; 16]);
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
