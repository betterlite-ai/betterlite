#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::finding::FindingStatus;
use betterlite_lens::models::job::Lens;

use crate::lens_access::tests::support::{Harness, admin, call, finding, lens, request};

fn seeded_lens() -> Lens {
    let mut stored = lens("l1");
    stored.findings = vec![finding("f1"), finding("f2")];
    stored
}

#[tokio::test]
async fn the_matching_finding_gets_the_status_and_reason() {
    let harness = Harness::new();
    harness.database.seed_lens(seeded_lens());
    let body = r#"{"status":"resolved","reason":"checked"}"#;
    let reply = call(
        harness.app(),
        request("PATCH", "/lens/l1/findings/f1", Some(&admin()), Some(body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    let [first, second] = updated.findings.as_slice() else {
        panic!("both findings stay");
    };
    assert_eq!(first.id, "f1");
    assert_eq!(first.status, FindingStatus::Resolved);
    assert_eq!(first.reason, "checked");
    assert_eq!(second.id, "f2");
    assert_eq!(second.status, FindingStatus::Open);
    assert_eq!(second.reason, "old");
}

#[tokio::test]
async fn a_status_only_request_clears_the_old_reason() {
    let harness = Harness::new();
    harness.database.seed_lens(seeded_lens());
    let body = r#"{"status":"dismissed"}"#;
    let reply = call(
        harness.app(),
        request("PATCH", "/lens/l1/findings/f1", Some(&admin()), Some(body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    assert_eq!(updated.findings[0].reason, "");
    assert_eq!(updated.findings[0].status, FindingStatus::Dismissed);
}

#[tokio::test]
async fn an_unknown_finding_changes_nothing() {
    let harness = Harness::new();
    harness.database.seed_lens(seeded_lens());
    let body = r#"{"status":"resolved","reason":"checked"}"#;
    let reply = call(
        harness.app(),
        request(
            "PATCH",
            "/lens/l1/findings/missing",
            Some(&admin()),
            Some(body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let unchanged: Lens = reply.json();
    assert_eq!(unchanged.findings[0].reason, "old");
    assert_eq!(unchanged.findings[0].status, FindingStatus::Open);
    let queries = harness.database.queries();
    assert!(!queries.iter().any(|query| query.contains("WITH previous")));
}

#[tokio::test]
async fn an_invalid_status_is_422() {
    let harness = Harness::new();
    harness.database.seed_lens(seeded_lens());
    let body = r#"{"status":"pending"}"#;
    let reply = call(
        harness.app(),
        request("PATCH", "/lens/l1/findings/f1", Some(&admin()), Some(body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].loc[0], "body");
}

#[tokio::test]
async fn a_missing_lens_is_404() {
    let harness = Harness::new();
    let body = r#"{"status":"resolved"}"#;
    let reply = call(
        harness.app(),
        request(
            "PATCH",
            "/lens/nope/findings/f1",
            Some(&admin()),
            Some(body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}
