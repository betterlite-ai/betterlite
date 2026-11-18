#![cfg(test)]

use axum::http::StatusCode;
use rstest::rstest;

use crate::roi_connections::gateway_key::MISSING_KEY;
use crate::roi_connections::routes::connections::{CONNECTIONS_TEST_PATH, NOT_READY};
use crate::roi_connections::test_estimator_access::{CANNOT_ACCESS, CANNOT_CONNECT};
use crate::roi_connections::tests::support::{Answer, admin, call, harness, post, stored};

fn models_ok() -> Answer {
    Answer::ok(r#"{"data":[{"id":"gpt-4o"}]}"#)
}

#[tokio::test]
async fn estimator_missing_key_is_a_409() {
    let harness = harness()
        .stored(stored("", &["acme/widgets"]))
        .models(&["gpt-4o"]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), MISSING_KEY);
    assert!(harness.wire.seen().is_empty());
}

#[tokio::test]
async fn estimator_master_key_is_the_fallback() {
    let harness = harness()
        .stored(stored("", &["acme/widgets"]))
        .master_key("master-key")
        .models(&["gpt-4o"])
        .answers(vec![models_ok(), Answer::ok("{}"), Answer::ok("{}")]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let seen = harness.wire.seen();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer master-key"));
}

#[tokio::test]
async fn estimator_not_ready_is_a_409() {
    let harness = harness();
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_READY);
    assert!(harness.wire.seen().is_empty());
}

#[tokio::test]
async fn estimator_status_is_a_409() {
    let harness = harness()
        .models(&["gpt-4o"])
        .answers(vec![Answer::status(500)]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), CANNOT_CONNECT);
}

#[tokio::test]
async fn estimator_missing_model_is_a_409() {
    let harness = harness()
        .models(&["gpt-4o"])
        .answers(vec![Answer::ok(r#"{"data":[{"id":"other-model"}]}"#)]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), CANNOT_ACCESS);
}

#[rstest]
#[case::not_json("not json")]
#[case::missing_data("{}")]
#[case::data_not_a_list(r#"{"data":{"id":"gpt-4o"}}"#)]
#[tokio::test]
async fn estimator_invalid_answer_is_a_409(#[case] body: &str) {
    let harness = harness()
        .models(&["gpt-4o"])
        .answers(vec![Answer::ok(body)]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), CANNOT_CONNECT);
}

#[tokio::test]
async fn github_http_error_is_a_502() {
    let harness = harness()
        .models(&["gpt-4o"])
        .answers(vec![models_ok(), Answer::status(404)]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        reply.detail(),
        "GitHub repository or organization not found. Check its name, token access, and API URL. (HTTP 404)"
    );
}

#[tokio::test]
async fn github_pull_error_is_a_502() {
    let harness = harness().models(&["gpt-4o"]).answers(vec![
        models_ok(),
        Answer::ok("{}"),
        Answer::status(500),
    ]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.detail(), "GitHub returned an error. (HTTP 500)");
}

#[tokio::test]
async fn github_credential_error_is_a_502() {
    let harness = harness()
        .stored(r#"{"github_api_url":"https://api.github.com","github_token":"bad\ntoken","estimator_key":"sk-estimator","repos":["acme/widgets"],"estimator_model":"gpt-4o"}"#)
        .models(&["gpt-4o"])
        .answers(vec![models_ok()]);
    let reply = call(harness.app(), post(CONNECTIONS_TEST_PATH, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        reply.detail(),
        "Could not reach GitHub. Check the API URL and network connection."
    );
}
