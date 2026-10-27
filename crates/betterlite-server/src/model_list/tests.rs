#![cfg(test)]
use crate::routes::router;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tower::ServiceExt as _;
const CONFIG: &str = "model_list:
  - {model_name: oracle-shared, litellm_params: {model: openai/oracle-first}}
  - {model_name: oracle-azure, litellm_params: {model: azure/oracle-azure}}
  - {model_name: oracle-shared, litellm_params: {model: azure/oracle-second}}
  - {model_name: oracle-bare, litellm_params: {model: gpt-4o-mini}}
";
const WILDCARD: &str = "model_list: [{model_name: openai/*, litellm_params: {model: openai/*}}]";
const LIST: &str = concat!(
    r#"{"data":[{"id":"oracle-shared","object":"model","created":1677610602,"owned_by":"openai"},"#,
    r#"{"id":"oracle-azure","object":"model","created":1677610602,"owned_by":"openai"},"#,
    r#"{"id":"oracle-bare","object":"model","created":1677610602,"owned_by":"openai"}],"object":"list"}"#
);
const BOOL_ISSUE: &str = r#""msg":"not a boolean","type":"bool_parsing"}"#;
async fn send(config: &str, uri: &str, header: &str) -> (u16, String) {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let model_list = loader.load(config).unwrap().model_list;
    let application = router(
        None,
        Arc::new(AtomicBool::new(false)),
        &model_list,
        Router::new(),
    );
    let request = Request::get(uri).header(header, "2023-06-01");
    let response = application
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}
#[rstest]
#[case::v1("/v1/models", LIST)]
#[case::bare("/models", LIST)]
#[case::expanded_scope("/models?scope=expand", LIST)]
#[case::flags_without_effect(
    "/v1/models?return_wildcard_routes=YES&include_model_access_groups=1&healthy_only=on&team_id=t",
    LIST
)]
#[case::last_repeated_value(
    "/v1/models?scope=x&scope=expand&include_metadata=1&include_metadata=f",
    LIST
)]
#[case::only_access_groups(
    "/v1/models?only_model_access_groups=TRUE",
    r#"{"data":[],"object":"list"}"#
)]
#[case::first_deployment(
    "/v1/models/oracle-shared",
    r#"{"id":"oracle-shared","object":"model","created":1677610602,"owned_by":"openai"}"#
)]
#[case::provider_prefix(
    "/models/oracle-azure?healthy_only=false",
    r#"{"id":"oracle-azure","object":"model","created":1677610602,"owned_by":"azure"}"#
)]
#[tokio::test]
async fn model_list_names_each_group_once_in_config_order(#[case] uri: &str, #[case] body: &str) {
    assert_eq!(send(CONFIG, uri, "x-other").await, (200, body.to_owned()));
}
#[rstest]
#[case::unknown_model("/v1/models/missing", 404, "model missing is not configured")]
#[case::comma_list("/models/a,b", 404, "model a,b is not configured")]
#[case::bad_scope("/v1/models?scope=nope", 400, "scope must be expand, got nope")]
#[case::metadata(
    "/v1/models?include_metadata=true",
    501,
    "betterlite does not support include_metadata yet"
)]
#[case::bare_model_provider(
    "/v1/models/oracle-bare",
    501,
    "betterlite does not support the model gpt-4o-mini without a provider prefix yet"
)]
#[tokio::test]
async fn model_list_errors_match_litellm_status(
    #[case] uri: &str,
    #[case] status: u16,
    #[case] detail: &str,
) {
    let body = format!(r#"{{"detail":"{detail}"}}"#);
    assert_eq!(send(CONFIG, uri, "x-other").await, (status, body));
}
#[rstest]
#[case::list(
    "/v1/models?healthy_only=%20true&include_metadata=&return_wildcard_routes=x&scope=nope",
    &format!(
        r#"{{"detail":[{{"loc":["query","return_wildcard_routes"],{BOOL_ISSUE},{{"loc":["query","include_metadata"],{BOOL_ISSUE},{{"loc":["query","healthy_only"],{BOOL_ISSUE}]}}"#
    )
)]
#[case::one_model(
    "/v1/models/oracle-missing?healthy_only=2&include_metadata=bad",
    &format!(r#"{{"detail":[{{"loc":["query","healthy_only"],{BOOL_ISSUE}]}}"#)
)]
#[tokio::test]
async fn model_list_flags_parse_like_pydantic(#[case] uri: &str, #[case] body: &str) {
    assert_eq!(send(CONFIG, uri, "x-other").await, (422, body.to_owned()));
}
#[rstest]
#[case::anthropic_format(CONFIG, "anthropic-version", "the Anthropic model list")]
#[case::wildcard(WILDCARD, "x-other", "the wildcard model openai/*")]
#[tokio::test]
async fn model_list_refuses_what_betterlite_lacks(
    #[case] config: &str,
    #[case] header: &str,
    #[case] feature: &str,
) {
    let body = format!(r#"{{"detail":"betterlite does not support {feature} yet"}}"#);
    assert_eq!(send(config, "/v1/models", header).await, (501, body));
}
