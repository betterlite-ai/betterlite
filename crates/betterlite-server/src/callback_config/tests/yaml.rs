use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use axum::http::StatusCode;
use rstest::rstest;

use super::support::{Harness, get_request, json, master_key, request_with_body};
use crate::callback_config::body::{callback_delete_request, config_yaml_request};

#[tokio::test]
async fn config_yaml_answers_hello_world() {
    let harness = Harness::new(BTreeMap::new());
    let request = request_with_body("GET", "/config/yaml", "{}");
    let (status, body) = harness.call(request, Some(master_key())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json(r#"{"hello": "world"}"#));
    assert_eq!(harness.reload_calls(), 0);
}

#[rstest]
#[case(
    "",
    r#"{"detail":[{"type":"missing","loc":["body"],"msg":"Field required","input":null}]}"#
)]
#[case(
    "null",
    r#"{"detail":[{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from","input":null}]}"#
)]
#[case(
    "[1]",
    r#"{"detail":[{"type":"model_attributes_type","loc":["body"],"msg":"Input should be a valid dictionary or object to extract fields from","input":[1]}]}"#
)]
#[case(
    "{bad",
    r#"{"detail":[{"type":"json_invalid","loc":["body"],"msg":"JSON decode error","input":null}]}"#
)]
#[tokio::test]
async fn config_yaml_answers_fastapi_validation_errors(
    #[case] payload: &str,
    #[case] expected: &str,
) {
    let harness = Harness::new(BTreeMap::new());
    let request = request_with_body("GET", "/config/yaml", payload);
    let (status, body) = harness.call(request, Some(master_key())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body, json(expected));
}

#[tokio::test]
async fn config_yaml_requires_authentication() {
    let harness = Harness::new(BTreeMap::new());
    let (status, _body) = harness.call(get_request("/config/yaml"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[rstest]
fn replays_the_callback_config_body_corpus(
    #[files("fuzz/corpus/callback_config_body/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("a corpus file");
    drop(callback_delete_request(&bytes));
    drop(config_yaml_request(&bytes));
}
