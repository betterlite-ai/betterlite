use betterlite_models::decisions::Json;

use crate::count_call::counter::{BedrockTokenCounter, TokenCountResponse};

use super::logs::{Captured, Kind, capture};
use super::support::{Reply, call, deployment, environment, fixture, user_messages};

const API_WARNING: &str = "Bedrock CountTokens API error";
const GENERIC_WARNING: &str = "Error calling Bedrock CountTokens API";

pub(crate) async fn count_api_failure_warning() {
    status_error_branch().await;
    non_200_branch().await;
    redirect_branch().await;
    unreadable_reply().await;
    non_object_reply().await;
    null_input_tokens().await;
    transport_failure().await;
}

fn key() -> Vec<(&'static str, Json)> {
    vec![("api_key", Json::Text("fixture-bearer".to_owned()))]
}

async fn failing(reply: Reply) -> (TokenCountResponse, Captured) {
    let messages = user_messages();
    let params = deployment(&key());
    let (logs, guard) = capture();
    let built = fixture(reply, environment(&[]));
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let outcome = counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    drop(guard);
    (outcome, logs)
}

async fn status_error_branch() {
    let (outcome, logs) = failing(Reply::with_headers(
        429,
        &[("x-fixture", "yes")],
        "rate limited",
    ))
    .await;
    assert!(outcome.error);
    assert_eq!(outcome.total_tokens, 0);
    assert_eq!(outcome.status_code, Some(429));
    assert_eq!(outcome.error_message.as_deref(), Some("rate limited"));
    assert_eq!(outcome.original_response, None);
    assert_eq!(
        logs.subjects(Kind::Error),
        ["HTTP error in CountTokens handler"]
    );
    assert_eq!(logs.subjects(Kind::Warn), [API_WARNING]);
    assert_eq!(logs.of(Kind::Warn)[0].status, Some(429));
}

async fn non_200_branch() {
    let (outcome, logs) = failing(Reply::status(201, "created")).await;
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(201));
    assert_eq!(outcome.error_message.as_deref(), Some("created"));
    assert_eq!(logs.subjects(Kind::Error), ["AWS Bedrock error"]);
    assert_eq!(logs.of(Kind::Warn)[0].status, Some(201));
}

async fn redirect_branch() {
    let (outcome, logs) = failing(Reply::status(302, "redirected")).await;
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(302));
    assert_eq!(outcome.error_message.as_deref(), Some("redirected"));
    assert_eq!(
        logs.subjects(Kind::Error),
        ["HTTP error in CountTokens handler"]
    );
    assert_eq!(logs.of(Kind::Warn)[0].status, Some(302));
}

async fn unreadable_reply() {
    let (outcome, logs) = failing(Reply::ok("not json")).await;
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(500));
    let message = outcome.error_message.as_deref().unwrap();
    assert!(message.starts_with("CountTokens processing error: "));
    assert_eq!(logs.subjects(Kind::Error), ["Error in CountTokens handler"]);
    assert_eq!(logs.subjects(Kind::Warn), [API_WARNING]);
    assert_eq!(logs.of(Kind::Warn)[0].status, Some(500));
}

async fn non_object_reply() {
    let (outcome, _logs) = failing(Reply::ok("[]")).await;
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(500));
    assert!(
        outcome
            .error_message
            .as_deref()
            .unwrap()
            .starts_with("CountTokens processing error: ")
    );
}

async fn null_input_tokens() {
    let (outcome, logs) = failing(Reply::ok(r#"{"inputTokens": null}"#)).await;
    assert!(outcome.error);
    assert_eq!(outcome.total_tokens, 0);
    assert_eq!(outcome.status_code, Some(500));
    assert_eq!(outcome.original_response, None);
    assert_eq!(
        outcome.error_message.as_deref(),
        Some("input_tokens is not an integer")
    );
    assert_eq!(logs.subjects(Kind::Warn), [GENERIC_WARNING]);
    assert!(logs.subjects(Kind::Error).is_empty());
}

async fn transport_failure() {
    let (outcome, logs) = failing(Reply::Failure).await;
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(500));
    assert!(
        outcome
            .error_message
            .as_deref()
            .unwrap()
            .starts_with("CountTokens processing error: ")
    );
    assert_eq!(logs.subjects(Kind::Error), ["Error in CountTokens handler"]);
    assert_eq!(logs.subjects(Kind::Warn), [API_WARNING]);
    assert_eq!(logs.of(Kind::Warn)[0].status, Some(500));
}
