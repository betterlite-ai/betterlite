#![cfg(test)]

use axum::http::StatusCode;
use betterlite_roi::settings::ROISettingsResponse;

use crate::roi_connections::routes::connections::CONNECTIONS_TEST_PATH;
use crate::roi_connections::tests::support::{Answer, admin, call, harness, post, stored};

#[tokio::test]
async fn success_reports_the_public_settings() {
    let harness = harness().models(&["gpt-4o"]).answers(vec![
        Answer::ok(r#"{"data":[{"id":"gpt-4o"}]}"#),
        Answer::ok("{}"),
        Answer::ok("{}"),
    ]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let response: ROISettingsResponse = reply.json();
    assert!(response.ready);
    assert!(response.has_estimator_key);
    assert!(response.has_github_token);
    assert_eq!(response.estimator_model, "gpt-4o");
    assert_eq!(response.available_models, ["gpt-4o"]);
    let seen = harness.wire.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].url, "http://litellm.internal/v1/models");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer sk-estimator")
    );
    assert_eq!(seen[1].url, "https://api.github.com/repos/acme/widgets");
    assert_eq!(seen[1].authorization.as_deref(), Some("Bearer gh-token"));
    assert_eq!(
        seen[2].url,
        "https://api.github.com/repos/acme/widgets/pulls"
    );
}

#[tokio::test]
async fn success_tests_every_configured_repository() {
    let harness = harness()
        .stored(&stored("sk-estimator", &["acme/one", "acme/two"]))
        .models(&["gpt-4o"])
        .answers(vec![
            Answer::ok(r#"{"data":[{"id":"gpt-4o"}]}"#),
            Answer::ok("{}"),
            Answer::ok("{}"),
            Answer::ok("{}"),
            Answer::ok("{}"),
        ]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let seen = harness.wire.seen();
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[1].url, "https://api.github.com/repos/acme/one");
    assert_eq!(seen[2].url, "https://api.github.com/repos/acme/one/pulls");
    assert_eq!(seen[3].url, "https://api.github.com/repos/acme/two");
    assert_eq!(seen[4].url, "https://api.github.com/repos/acme/two/pulls");
    assert!(
        seen[2]
            .query
            .contains(&("state".to_owned(), "closed".to_owned()))
    );
    assert!(
        seen[2]
            .query
            .contains(&("per_page".to_owned(), "1".to_owned()))
    );
}
