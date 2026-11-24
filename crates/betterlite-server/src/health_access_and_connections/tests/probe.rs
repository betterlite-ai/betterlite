use std::future::pending;
use std::time::Duration;

use betterlite_models::decisions::Json;

use crate::health_access_and_connections::connection_state::{SELF_PROMPT, timed_probe};
use crate::health_check_entry::{
    HealthCheckFailure, HealthCheckOutcome, HealthEntryError, RawRequestRecord,
};
use crate::health_probe_modes::response::HealthCheckResponse;

use super::support::{object, text};

const TIMEOUT: Duration = Duration::from_secs(5);

fn failure(error: &str, raw_request: Option<RawRequestRecord>) -> HealthCheckOutcome {
    HealthCheckOutcome::Failed(Box::new(HealthCheckFailure {
        error: error.to_owned(),
        raw_request,
        exception: HealthEntryError::ModelNotSet,
    }))
}

#[test]
fn the_probe_prompt_is_litellms_own() {
    assert_eq!(SELF_PROMPT, "test from litellm");
}

#[tokio::test]
async fn a_healthy_outcome_carries_the_headers_the_provider_sent() {
    let response = HealthCheckResponse {
        remaining_requests: Some("10".to_owned()),
        remaining_tokens: Some("20".to_owned()),
        region: None,
    };
    let result = timed_probe(async { HealthCheckOutcome::Healthy(response) }, TIMEOUT)
        .await
        .expect("a healthy result encodes");
    assert_eq!(
        result,
        object(vec![
            ("x-ratelimit-remaining-requests", text("10")),
            ("x-ratelimit-remaining-tokens", text("20")),
        ])
    );
}

#[tokio::test]
async fn a_healthy_outcome_without_headers_is_empty() {
    let empty = HealthCheckResponse::default();
    let result = timed_probe(async { HealthCheckOutcome::Healthy(empty) }, TIMEOUT)
        .await
        .expect("an empty healthy result encodes");
    assert!(result.is_empty());
}

#[tokio::test]
async fn an_ordinary_failure_carries_its_raw_request() {
    let raw = RawRequestRecord {
        raw_request_api_base: Some("https://api".to_owned()),
        raw_request_body: Some(Json::Object(object(vec![("model", text("gpt"))]))),
        raw_request_headers: None,
        error: Some("boom".to_owned()),
    };
    let outcome = failure("model not set\n stack trace: model not set", Some(raw));
    let result = timed_probe(async { outcome }, TIMEOUT)
        .await
        .expect("a failure result encodes");
    assert_eq!(
        result.get("error").cloned(),
        Some(text("model not set\n stack trace: model not set"))
    );
    assert_eq!(
        result.get("raw_request_typed_dict").cloned(),
        Some(Json::Object(object(vec![
            ("raw_request_api_base", text("https://api")),
            (
                "raw_request_body",
                Json::Object(object(vec![("model", text("gpt"))]))
            ),
            ("raw_request_headers", Json::Null),
            ("error", text("boom")),
        ])))
    );
}

#[tokio::test]
async fn an_ordinary_failure_without_a_raw_request_keeps_the_key_null() {
    let outcome = failure("model not set\n stack trace: model not set", None);
    let result = timed_probe(async { outcome }, TIMEOUT)
        .await
        .expect("a failure result encodes");
    assert!(result.contains_key("error"));
    assert_eq!(result.get("raw_request_typed_dict"), Some(&Json::Null));
}

#[tokio::test]
async fn the_missing_mode_failure_leaves_the_raw_request_key_out() {
    let outcome = failure(
        "error:model not set. Missing `mode`.  \nstacktrace: model not set",
        None,
    );
    let result = timed_probe(async { outcome }, TIMEOUT)
        .await
        .expect("a failure result encodes");
    assert!(result.contains_key("error"));
    assert!(!result.contains_key("raw_request_typed_dict"));
}

#[tokio::test(start_paused = true)]
async fn a_probe_over_the_timeout_is_the_timeout_result() {
    let result = timed_probe(pending::<HealthCheckOutcome>(), Duration::from_secs(60))
        .await
        .expect("a timeout result encodes");
    assert_eq!(result, object(vec![("error", text("Timeout exceeded"))]));
}
