use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use rstest::rstest;

use super::support::{DetailReply, FakeStore, LocReply, ValidationReply, call, master, member};
use crate::workflow_runs::models::{RunCreateData, WorkflowRunRow};

const CREATE: &str = "/v1/workflows/runs";

#[tokio::test]
async fn create_stores_the_caller_token_and_json_strings() {
    let store = Arc::new(FakeStore::default());
    let body = r#"{"workflow_type":"review","input":{"pr":7},"metadata":{"team":"core"}}"#;
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::POST,
        CREATE,
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowRunRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.created_by.as_deref(), Some("hash-1"));
    assert_eq!(row.status, "pending");
    assert_eq!(row.events, None);
    assert_eq!(
        store.recorded().created,
        vec![RunCreateData {
            workflow_type: "review".to_owned(),
            created_by: Some("hash-1".to_owned()),
            input: Some(r#"{"pr":7}"#.to_owned()),
            metadata: Some(r#"{"team":"core"}"#.to_owned()),
        }]
    );
}

#[tokio::test]
async fn create_without_input_or_metadata_stores_none() {
    let store = Arc::new(FakeStore::default());
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::POST,
        CREATE,
        r#"{"workflow_type":"review"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let created = store.recorded().created;
    assert_eq!(created.len(), 1);
    assert_eq!(created.first().unwrap().input, None);
    assert_eq!(created.first().unwrap().metadata, None);
}

#[tokio::test]
async fn create_by_the_master_key_stores_the_alias_as_owner() {
    let store = Arc::new(FakeStore::default());
    let reply = call(
        Some(&store),
        Some(&master()),
        Method::POST,
        CREATE,
        r#"{"workflow_type":"review"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let row: WorkflowRunRow = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(row.created_by.as_deref(), Some("litellm_proxy_master_key"));
}

#[tokio::test]
async fn create_without_a_database_is_a_500_with_the_db_error() {
    let reply = call(
        None,
        Some(&member("hash-1")),
        Method::POST,
        CREATE,
        r#"{"workflow_type":"review"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(
        detail.detail,
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[tokio::test]
async fn create_store_failure_is_a_500_without_the_cause() {
    let store = Arc::new(FakeStore {
        state: Mutex::default(),
        failing: true,
        vanish_on_update: false,
    });
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::POST,
        CREATE,
        r#"{"workflow_type":"review"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let detail: DetailReply = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(detail.detail, "Internal server error");
}

#[tokio::test]
async fn create_without_a_caller_is_unauthenticated() {
    let store = Arc::new(FakeStore::default());
    let reply = call(
        Some(&store),
        None,
        Method::POST,
        CREATE,
        r#"{"workflow_type":"review"}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(store.recorded().created.is_empty());
}

#[rstest]
#[case::missing_type("{}", "missing", &["body", "workflow_type"])]
#[case::type_is_a_number(r#"{"workflow_type":3}"#, "value_error", &["body", "workflow_type"])]
#[case::input_is_a_list(r#"{"workflow_type":"a","input":[1]}"#, "value_error", &["body", "input"])]
#[case::metadata_is_a_string(r#"{"workflow_type":"a","metadata":"x"}"#, "value_error", &["body", "metadata"])]
#[case::not_json("not json", "json_invalid", &["body"])]
#[case::second_value(r#"{"workflow_type":"a"} {}"#, "json_invalid", &["body"])]
#[tokio::test]
async fn create_rejects_a_body_of_the_wrong_shape(
    #[case] body: &str,
    #[case] issue_type: &str,
    #[case] loc: &[&str],
) {
    let store = Arc::new(FakeStore::default());
    let reply = call(
        Some(&store),
        Some(&member("hash-1")),
        Method::POST,
        CREATE,
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let parsed: ValidationReply = serde_json::from_str(&reply.body).unwrap();
    let issue = parsed.detail.first().unwrap();
    assert_eq!(issue.issue_type, issue_type);
    let expected: Vec<LocReply> = loc
        .iter()
        .map(|name| LocReply::Name((*name).to_owned()))
        .collect();
    assert_eq!(issue.loc, expected);
    assert!(store.recorded().created.is_empty());
}

#[tokio::test]
async fn create_with_a_bad_body_and_no_database_is_a_422() {
    let reply = call(None, Some(&member("hash-1")), Method::POST, CREATE, "{}").await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}
