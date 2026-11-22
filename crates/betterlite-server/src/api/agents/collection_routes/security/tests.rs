#![cfg(test)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use betterlite_core::agents::json::AgentJson as JsonValue;
use rstest::rstest;

use crate::api::agents::collection_routes::AGENTS_PATH;
use crate::api::agents::collection_routes::guards::MISSING_KEY_DETAIL;
use crate::api::agents::collection_routes::merge::TEMPLATE_FIELD;
use crate::api::agents::collection_routes::tests::support::{
    FakePreCall, FakeProcessing, admin, call, json_object, json_text, key, master_key, query,
    request, router, user,
};
use crate::auth::auth_identity::AuthContext;

fn list_uri(template: &str) -> String {
    let encoded = query(&[(TEMPLATE_FIELD, template)]);
    format!("{AGENTS_PATH}?{encoded}")
}

async fn send(
    pre_call: Arc<FakePreCall>,
    processing: Arc<FakeProcessing>,
    request: Request<Body>,
) -> (StatusCode, JsonValue) {
    call(router(pre_call, processing), request).await
}

#[rstest]
#[case::create(Method::POST, AGENTS_PATH)]
#[case::list(Method::GET, AGENTS_PATH)]
#[case::get(Method::GET, "/v1beta/agents/model-a")]
#[case::delete(Method::DELETE, "/v1beta/agents/model-a")]
#[case::versions(Method::GET, "/v1beta/agents/model-a/versions")]
#[tokio::test]
async fn a_non_admin_without_a_key_is_refused_before_the_call(
    #[case] method: Method,
    #[case] path: &str,
) {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json_object("{}"));
    let (status, body) = send(
        Arc::clone(&pre_call),
        Arc::clone(&processing),
        request(method, path, None, user(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let detail = body.as_object().unwrap();
    assert_eq!(detail.len(), 1);
    assert_eq!(detail.get("detail"), Some(&json_text(MISSING_KEY_DETAIL)));
    assert!(pre_call.calls().is_empty());
    assert!(processing.calls().is_empty());
    assert!(processing.failures().is_empty());
}

// LiteLLM reads the body before it calls the guard, so a body it cannot parse answers 400 even without a key
#[tokio::test]
async fn a_malformed_body_is_refused_before_the_credential_guard() {
    let pre_call = FakePreCall::passing();
    let (status, body) = send(
        Arc::clone(&pre_call),
        FakeProcessing::answering(json_object("{}")),
        request(Method::POST, AGENTS_PATH, Some("{not json"), user(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["param"], json_text("request_body"));
    assert!(pre_call.calls().is_empty());
}

#[tokio::test]
async fn the_401_body_is_the_plain_detail_envelope() {
    let (status, body) = send(
        FakePreCall::passing(),
        FakeProcessing::answering(json_object("{}")),
        request(Method::GET, AGENTS_PATH, None, user(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let object = body.as_object().unwrap();
    assert_eq!(object.len(), 1);
    assert_eq!(object.get("detail"), Some(&json_text(MISSING_KEY_DETAIL)));
}

async fn reaches_the_call(caller: AuthContext) {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json_object("{}"));
    let (status, _body) = send(
        Arc::clone(&pre_call),
        Arc::clone(&processing),
        request(Method::GET, AGENTS_PATH, None, caller, key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pre_call.calls().len(), 1);
    let calls = processing.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].data.get("custom_llm_provider"),
        Some(&json_text("gemini"))
    );
    assert!(!calls[0].data.contains_key("api_key"));
}

#[tokio::test]
async fn a_role_admin_without_a_key_reaches_the_call() {
    reaches_the_call(admin()).await;
}

#[tokio::test]
async fn the_master_key_without_a_key_reaches_the_call() {
    reaches_the_call(master_key()).await;
}

#[tokio::test]
async fn a_body_api_key_lets_a_non_admin_through() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json_object("{}"));
    let (status, _body) = send(
        Arc::clone(&pre_call),
        Arc::clone(&processing),
        request(
            Method::POST,
            AGENTS_PATH,
            Some(r#"{"name":"agent","api_key":"AIza"}"#),
            user(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pre_call.calls().len(), 1);
    assert_eq!(
        processing.calls()[0].data.get("api_key"),
        Some(&json_text("AIza"))
    );
}

#[tokio::test]
async fn a_body_template_api_key_lets_a_non_admin_through() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json_object("{}"));
    let (status, _body) = send(
        Arc::clone(&pre_call),
        Arc::clone(&processing),
        request(
            Method::POST,
            AGENTS_PATH,
            Some(r#"{"litellm_params_template":{"api_key":"AIza"}}"#),
            user(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        processing.calls()[0].data.get("api_key"),
        Some(&json_text("AIza"))
    );
}

#[tokio::test]
async fn a_query_template_api_key_lets_a_non_admin_through() {
    let pre_call = FakePreCall::passing();
    let processing = FakeProcessing::answering(json_object("{}"));
    let (status, _body) = send(
        Arc::clone(&pre_call),
        Arc::clone(&processing),
        request(
            Method::GET,
            &list_uri(r#"{"api_key":"AIza"}"#),
            None,
            user(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pre_call.calls().len(), 1);
    assert_eq!(
        processing.calls()[0].data.get("api_key"),
        Some(&json_text("AIza"))
    );
}

#[tokio::test]
async fn a_flat_api_key_query_parameter_does_not_let_a_non_admin_through() {
    let uri = format!("{AGENTS_PATH}?api_key=AIza");
    let (status, _body) = send(
        FakePreCall::passing(),
        FakeProcessing::answering(json_object("{}")),
        request(Method::GET, &uri, None, user(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[rstest]
#[case::empty(r#"{"api_key": ""}"#)]
#[case::null(r#"{"api_key": null}"#)]
#[case::false_value(r#"{"api_key": false}"#)]
#[case::zero(r#"{"api_key": 0}"#)]
#[case::empty_object(r#"{"api_key": {}}"#)]
#[tokio::test]
async fn a_falsy_body_api_key_is_refused(#[case] body: &str) {
    let pre_call = FakePreCall::passing();
    let (status, _body) = send(
        Arc::clone(&pre_call),
        FakeProcessing::answering(json_object("{}")),
        request(Method::POST, AGENTS_PATH, Some(body), user(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(pre_call.calls().is_empty());
}

#[tokio::test]
async fn a_query_template_cannot_override_the_fixed_provider_on_admin_list() {
    let processing = FakeProcessing::answering(json_object("{}"));
    let (status, _body) = send(
        FakePreCall::passing(),
        Arc::clone(&processing),
        request(
            Method::GET,
            &list_uri(r#"{"custom_llm_provider":"vertex","api_key":"AIza"}"#),
            None,
            master_key(),
            key(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        processing.calls()[0].data.get("custom_llm_provider"),
        Some(&json_text("gemini"))
    );
}

#[tokio::test]
async fn the_item_route_keeps_the_path_name_over_a_query_template() {
    let processing = FakeProcessing::answering(json_object("{}"));
    let encoded = query(&[(TEMPLATE_FIELD, r#"{"name":"other","api_key":"AIza"}"#)]);
    let uri = format!("/v1beta/agents/model-a?{encoded}");
    let (status, _body) = send(
        FakePreCall::passing(),
        Arc::clone(&processing),
        request(Method::GET, &uri, None, master_key(), key()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let data = &processing.calls()[0].data;
    assert_eq!(data.get("name"), Some(&json_text("model-a")));
    assert_eq!(data.get("api_key"), Some(&json_text("AIza")));
}
