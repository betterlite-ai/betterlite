#![cfg(test)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use betterlite_core::agents::json::AgentJson as JsonValue;
use serde_json::json;

use crate::api::agents::collection_routes::AGENTS_PATH;
use crate::api::agents::collection_routes::merge::TEMPLATE_FIELD;
use crate::api::agents::collection_routes::tests::support::{
    FakePreCall, FakeProcessing, TEAM, VERSION, admin, call, json_map, key, query, request, router,
};
use crate::pre_call::failure::{PreCallFailure, ProxyFailure};
use crate::pre_call::route_type::RouteType;

fn failure() -> PreCallFailure {
    PreCallFailure::Proxy(ProxyFailure {
        message: "boom".to_owned(),
        error_type: "internal_server_error".to_owned(),
        param: None,
        code: 500,
    })
}

#[tokio::test]
async fn create_runs_the_pre_call_with_the_body_and_the_call_after_it() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json!({"name": "agent"}));
    let (status, body) = call(
        router(Arc::clone(&pre_call), Arc::clone(&processing)),
        request(
            Method::POST,
            AGENTS_PATH,
            Some(r#"{"name":"agent","service_account":"sa@example.test"}"#),
            admin(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"name": "agent"}));
    let pre_calls = pre_call.calls();
    assert_eq!(pre_calls.len(), 1);
    assert_eq!(
        JsonValue::Object(pre_calls[0].data.clone()),
        json!({"name": "agent", "service_account": "sa@example.test", "custom_llm_provider": "gemini"})
    );
    assert_eq!(pre_calls[0].route_type, RouteType::CreateAgent);
    assert_eq!(pre_calls[0].version.as_deref(), Some(VERSION));
    assert_eq!(pre_calls[0].model, None);
    assert!(!pre_calls[0].skip_guardrails);
    assert_eq!(pre_calls[0].team.as_deref(), Some(TEAM));
    let calls = processing.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].data, pre_calls[0].data);
    assert_eq!(calls[0].route_type, RouteType::CreateAgent);
    assert_eq!(calls[0].version, VERSION);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].path, AGENTS_PATH);
    assert_eq!(calls[0].team.as_deref(), Some(TEAM));
}

#[tokio::test]
async fn create_merges_the_body_template_with_body_precedence_and_drops_it() {
    let pre_call = FakePreCall::passing();
    let (status, _body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        request(
            Method::POST,
            AGENTS_PATH,
            Some(r#"{"name":"agent","litellm_params_template":{"name":"other","api_key":"AIza"}}"#),
            admin(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let data = &pre_call.calls()[0].data;
    assert_eq!(data.get("name"), Some(&json!("agent")));
    assert_eq!(data.get("api_key"), Some(&json!("AIza")));
    assert!(!data.contains_key(TEMPLATE_FIELD));
}

#[tokio::test]
async fn create_lets_the_template_pick_the_provider_when_the_body_does_not() {
    let pre_call = FakePreCall::passing();
    let (status, _body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        request(
            Method::POST,
            AGENTS_PATH,
            Some(r#"{"litellm_params_template":{"custom_llm_provider":"vertex"}}"#),
            admin(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        pre_call.calls()[0].data.get("custom_llm_provider"),
        Some(&json!("vertex"))
    );
}

#[tokio::test]
async fn create_of_an_empty_body_still_defaults_the_provider() {
    let pre_call = FakePreCall::passing();
    let (status, _body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        request(Method::POST, AGENTS_PATH, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        JsonValue::Object(pre_call.calls()[0].data.clone()),
        json!({"custom_llm_provider": "gemini"})
    );
}

#[tokio::test]
async fn create_reads_only_the_body_template_and_ignores_the_query_one() {
    let pre_call = FakePreCall::passing();
    let encoded = query(&[(TEMPLATE_FIELD, r#"{"api_key":"query-key"}"#)]);
    let uri = format!("{AGENTS_PATH}?{encoded}");
    let (status, _body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        request(
            Method::POST,
            &uri,
            Some(r#"{"name":"agent"}"#),
            admin(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let data = &pre_call.calls()[0].data;
    assert!(!data.contains_key("api_key"));
    assert_eq!(data.get("custom_llm_provider"), Some(&json!("gemini")));
}

// the route hands the request media type to _read_request_body, so a form body decodes into its fields
#[tokio::test]
async fn create_reads_a_form_body_through_the_request_content_type() {
    let pre_call = FakePreCall::passing();
    let form_request = Request::builder()
        .method(Method::POST)
        .uri(AGENTS_PATH)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .extension(admin())
        .extension(key())
        .body(Body::from("name=agent&api_key=AIza"))
        .unwrap();
    let (status, _body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        form_request,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let data = &pre_call.calls()[0].data;
    assert_eq!(data.get("name"), Some(&json!("agent")));
    assert_eq!(data.get("api_key"), Some(&json!("AIza")));
    assert_eq!(data.get("custom_llm_provider"), Some(&json!("gemini")));
}

#[tokio::test]
async fn create_rejects_a_body_that_is_not_json_before_the_pre_call() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json!({}));
    let (status, body) = call(
        router(Arc::clone(&pre_call), Arc::clone(&processing)),
        request(Method::POST, AGENTS_PATH, Some("{not json"), admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["type"], json!("invalid_request_error"));
    assert_eq!(body["error"]["param"], json!("request_body"));
    assert_eq!(body["error"]["code"], json!("400"));
    assert!(pre_call.calls().is_empty());
    assert!(processing.calls().is_empty());
}

#[tokio::test]
async fn create_rejects_a_body_that_is_not_an_object() {
    let pre_call = FakePreCall::passing();
    let (status, body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        request(Method::POST, AGENTS_PATH, Some("[1,2]"), admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["param"], json!("request_body"));
    assert!(pre_call.calls().is_empty());
}

#[tokio::test]
async fn list_starts_from_the_provider_and_merges_the_query_template() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json!({"agents": []}));
    let encoded = query(&[(TEMPLATE_FIELD, r#"{"api_key":"AIza","page":"2"}"#)]);
    let uri = format!("{AGENTS_PATH}?{encoded}");
    let (status, body) = call(
        router(Arc::clone(&pre_call), Arc::clone(&processing)),
        request(Method::GET, &uri, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"agents": []}));
    let pre_calls = pre_call.calls();
    assert_eq!(pre_calls[0].route_type, RouteType::ListAgents);
    assert_eq!(
        JsonValue::Object(pre_calls[0].data.clone()),
        json!({"custom_llm_provider": "gemini", "api_key": "AIza", "page": "2"})
    );
    assert_eq!(processing.calls()[0].route_type, RouteType::ListAgents);
    assert_eq!(processing.calls()[0].method, "GET");
}

#[tokio::test]
async fn list_without_a_query_string_sends_only_the_provider() {
    let pre_call = FakePreCall::passing();
    let (status, _body) = call(
        router(Arc::clone(&pre_call), FakeProcessing::answering(json!({}))),
        request(Method::GET, AGENTS_PATH, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        JsonValue::Object(pre_call.calls()[0].data.clone()),
        json!({"custom_llm_provider": "gemini"})
    );
}

#[tokio::test]
async fn get_names_the_agent_from_the_path() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json!({"name": "model-a"}));
    let uri = "/v1beta/agents/model-a";
    let (status, _body) = call(
        router(Arc::clone(&pre_call), Arc::clone(&processing)),
        request(Method::GET, uri, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        JsonValue::Object(pre_call.calls()[0].data.clone()),
        json!({"name": "model-a", "custom_llm_provider": "gemini"})
    );
    assert_eq!(pre_call.calls()[0].route_type, RouteType::GetAgent);
    assert_eq!(processing.calls()[0].path, uri);
    assert_eq!(processing.calls()[0].method, "GET");
}

#[tokio::test]
async fn delete_names_the_agent_and_uses_the_delete_route_type() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json!({}));
    let (status, _body) = call(
        router(Arc::clone(&pre_call), Arc::clone(&processing)),
        request(
            Method::DELETE,
            "/v1beta/agents/model-a",
            None,
            admin(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pre_call.calls()[0].route_type, RouteType::DeleteAgent);
    assert_eq!(
        JsonValue::Object(pre_call.calls()[0].data.clone()),
        json!({"name": "model-a", "custom_llm_provider": "gemini"})
    );
    assert_eq!(processing.calls()[0].method, "DELETE");
}

#[tokio::test]
async fn versions_names_the_agent_and_uses_the_versions_route_type() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json!({}));
    let uri = "/v1beta/agents/model-a/versions";
    let (status, _body) = call(
        router(Arc::clone(&pre_call), Arc::clone(&processing)),
        request(Method::GET, uri, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pre_call.calls()[0].route_type, RouteType::ListAgentVersions);
    assert_eq!(
        JsonValue::Object(pre_call.calls()[0].data.clone()),
        json!({"name": "model-a", "custom_llm_provider": "gemini"})
    );
    assert_eq!(processing.calls()[0].path, uri);
}

#[tokio::test]
async fn a_pre_call_failure_goes_to_the_exception_mapping_with_the_held_data() {
    let pre_call = FakePreCall::failing(failure());
    let processing = FakeProcessing::answering(json!({}));
    let (status, body) = call(
        router(pre_call, Arc::clone(&processing)),
        request(Method::GET, AGENTS_PATH, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["handled"], json!(true));
    assert_eq!(body["message"], json!("boom"));
    assert!(processing.calls().is_empty());
    let failures = processing.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].failure, failure());
    assert_eq!(
        JsonValue::Object(failures[0].data.clone()),
        json!({"custom_llm_provider": "gemini"})
    );
}

#[tokio::test]
async fn a_processing_failure_goes_to_the_exception_mapping_with_the_post_pre_call_data() {
    let replacement =
        json_map(r#"{"name":"agent","custom_llm_provider":"gemini","tag":"replaced"}"#);
    let pre_call = FakePreCall::replacing(replacement.clone());
    let processing = FakeProcessing::failing(failure());
    let (status, body) = call(
        router(pre_call, Arc::clone(&processing)),
        request(
            Method::POST,
            AGENTS_PATH,
            Some(r#"{"name":"agent"}"#),
            admin(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["handled"], json!(true));
    let failures = processing.failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].failure, failure());
    assert_eq!(failures[0].data, replacement);
}

#[tokio::test]
async fn the_processing_answer_is_the_route_answer() {
    let processing = FakeProcessing::answering(json!({"agents": [{"name": "model-a"}]}));
    let (status, body) = call(
        router(FakePreCall::passing(), processing),
        request(Method::GET, AGENTS_PATH, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"agents": [{"name": "model-a"}]}));
}

#[tokio::test]
async fn an_unknown_method_on_the_collection_path_is_not_allowed() {
    let (status, _body) = call(
        router(FakePreCall::passing(), FakeProcessing::answering(json!({}))),
        request(Method::PUT, AGENTS_PATH, None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn an_unknown_agent_subpath_is_not_found() {
    let (status, _body) = call(
        router(FakePreCall::passing(), FakeProcessing::answering(json!({}))),
        request(Method::GET, "/v1beta/agents/a/b", None, admin(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
