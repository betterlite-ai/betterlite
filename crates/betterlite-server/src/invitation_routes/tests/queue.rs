#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode};
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;
use serde_json::json;
use tower::ServiceExt as _;

use super::support::{
    FakeCompletion, FakeHook, Reply, key_caller, master, object, parse, queue_router, queue_state,
    scheduler, send, send_with, userless_admin,
};
use crate::invitation_routes::state::{QueueFailureHook, QueueRouteState};
use crate::pre_call::rewrites::CliOverrides;
use crate::sse_data_generator::types::{FailureKind, FailureParam, FailureType, StreamFailure};
use crate::stream_chunks::DONE_FRAME;
use crate::stream_chunks::StreamItem;

const NO_PRIORITY: &str = "Authentication Error, the request carries no priority between 0 and 255";

fn chat_body(priority: i64) -> String {
    format!(
        r#"{{"model":"body-model","messages":[{{"role":"user","content":"hi"}}],"priority":{priority}}}"#
    )
}

fn key_fields() -> &'static str {
    r#"{"api_key":"plain-key","key_alias":"alias-1","user_id":"key-user","team_id":"team-1","object_permission_id":"op-1","team_object_permission_id":"top-1","metadata":{"logging":[{"callback_name":"x"}],"secret_manager_settings":{"a":1},"keep":true}}"#
}

fn replacement(code: i64) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "replaced".to_owned(),
        error_type: ProxyErrorTypes::AuthError.to_string(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(code),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

#[tokio::test]
async fn queue_without_a_router_answers_500() {
    let router = queue_router(QueueRouteState::default());
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let body = parse(&text);
    assert_eq!(body["error"]["message"], "No models configured on proxy");
    assert_eq!(body["error"]["type"], "auth_error");
    assert_eq!(body["error"]["param"], "None");
    assert_eq!(body["error"]["code"], "500");
}

#[tokio::test]
async fn queue_refuses_a_body_without_a_priority() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        r#"{"model":"m","messages":[]}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(parse(&text)["error"]["message"], NO_PRIORITY);
    assert_eq!(parse(&text)["error"]["param"], "None");
    assert_eq!(parse(&text)["error"]["code"], "400");
    assert!(completion.calls().is_empty());
}

#[rstest]
#[case::byte_overflow(r#"{"model":"m","priority":256}"#)]
#[case::negative(r#"{"model":"m","priority":-1}"#)]
#[case::float(r#"{"model":"m","priority":1.5}"#)]
#[case::text(r#"{"model":"m","priority":"3"}"#)]
#[case::null(r#"{"model":"m","priority":null}"#)]
#[tokio::test]
async fn queue_refuses_every_priority_a_byte_cannot_hold(#[case] body: &str) {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(parse(&text)["error"]["message"], NO_PRIORITY, "{body}");
    assert!(completion.calls().is_empty(), "{body}");
}

#[rstest]
#[case::zero(0_i64, "0")]
#[case::maximum(255_i64, "255")]
#[tokio::test]
async fn queue_accepts_the_edges_of_a_byte_priority(#[case] priority: i64, #[case] header: &str) {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, headers, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(priority),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-litellm-priority"], header);
}

#[tokio::test]
async fn queue_builds_the_request_like_litellm() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let body = r#"{"model":"body-model","messages":[{"role":"user","content":"hi"}],"priority":5,"metadata":{"team":"x"},"stream":false,"user":"given","_litellm_strip_stream_usage":false}"#;
    let request = Request::builder()
        .method("POST")
        .uri("/queue/chat/completions?model=query-model")
        .header("content-type", "application/json")
        .header("host", "proxy.test")
        .header("authorization", "Bearer secret")
        .header("x-dup", "a")
        .header("x-dup", "b")
        .extension(key_caller(key_fields()))
        .body(Body::from(body.to_owned()))
        .expect("a request");
    let (status, _, _) = send_with(&router, request).await;
    assert_eq!(status, StatusCode::OK);
    let seen = completion.seen();
    assert_eq!(seen["model"], "query-model");
    assert_eq!(seen["user"], "given");
    assert!(seen.get("_litellm_strip_stream_usage").is_none());
    assert_proxy_server_request(&seen);
    assert_metadata(&seen);
}

fn assert_proxy_server_request(seen: &JsonMap) {
    assert_eq!(
        seen["proxy_server_request"]["url"],
        "http://proxy.test/queue/chat/completions?model=query-model"
    );
    assert_eq!(seen["proxy_server_request"]["method"], "POST");
    assert_eq!(
        seen["proxy_server_request"]["headers"]["authorization"],
        "Bearer secret"
    );
    assert_eq!(seen["proxy_server_request"]["body"]["model"], "body-model");
    assert!(
        seen["proxy_server_request"]["body"]
            .get("proxy_server_request")
            .is_none()
    );
}

fn assert_metadata(seen: &JsonMap) {
    assert_eq!(seen["metadata"]["team"], "x");
    assert_eq!(seen["metadata"]["user_api_key"], "plain-key");
    assert_eq!(seen["metadata"]["user_api_key_hash"], "plain-key");
    assert_eq!(
        seen["metadata"]["user_api_key_metadata"],
        parse(r#"{"keep": true}"#)
    );
    assert_eq!(seen["metadata"]["headers"]["host"], "proxy.test");
    assert_eq!(seen["metadata"]["headers"]["x-dup"], "a, b");
    assert_eq!(
        seen["metadata"]["endpoint"],
        "http://proxy.test/queue/chat/completions?model=query-model"
    );
    assert!(seen["metadata"]["headers"].get("authorization").is_none());
    assert_eq!(seen["metadata"]["user_api_key_alias"], "alias-1");
    assert_eq!(seen["metadata"]["user_api_key_user_id"], "key-user");
    assert_eq!(seen["metadata"]["user_api_key_team_id"], "team-1");
    assert_eq!(
        seen["metadata"]["user_api_key_object_permission_id"],
        "op-1"
    );
    assert_eq!(
        seen["metadata"]["user_api_key_team_object_permission_id"],
        "top-1"
    );
}

#[tokio::test]
async fn queue_replaces_a_metadata_that_is_not_an_object() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let body = r#"{"model":"m","priority":1,"messages":[],"metadata":"text"}"#;
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(key_caller(key_fields())),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    let seen = completion.seen();
    assert!(seen["metadata"].is_object());
    assert_eq!(seen["metadata"]["user_api_key"], "plain-key");
}

#[tokio::test]
async fn queue_skips_a_header_value_that_is_not_utf8() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let opaque = HeaderValue::from_bytes(b"\xff\xfe").expect("an opaque header value");
    let request = Request::builder()
        .method("POST")
        .uri("/queue/chat/completions")
        .header("content-type", "application/json")
        .header("host", "proxy.test")
        .header("x-opaque", opaque)
        .extension(master())
        .body(Body::from(chat_body(1)))
        .expect("a request");
    let response = router.oneshot(request).await.expect("a response");
    assert_eq!(response.status(), StatusCode::OK);
    let seen = completion.seen();
    assert!(seen["metadata"]["headers"].get("x-opaque").is_none());
    assert_eq!(seen["metadata"]["headers"]["host"], "proxy.test");
}

#[tokio::test]
async fn queue_takes_the_user_from_the_key_when_the_body_has_none() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let body = r#"{"model":"m","priority":1,"user":null}"#;
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(key_caller(key_fields())),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completion.seen()["user"], "key-user");
}

#[tokio::test]
async fn queue_leaves_the_user_unset_for_an_identityless_call() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(userless_admin()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(completion.seen().get("user").is_none());
}

#[tokio::test]
async fn queue_fills_the_master_keys_user_with_the_proxy_admin_name() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completion.seen()["user"], "default_user_id");
}

#[tokio::test]
async fn queue_carries_the_master_keys_alias_and_user_id_in_metadata() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    let seen = completion.seen();
    assert_eq!(seen["metadata"]["user_api_key"], "litellm_proxy_master_key");
    assert_eq!(
        seen["metadata"]["user_api_key_hash"],
        "litellm_proxy_master_key"
    );
    assert_eq!(seen["metadata"]["user_api_key_user_id"], "default_user_id");
}

#[rstest]
#[case::completion_model(
    Some("completion-model"),
    Some("cli-model"),
    Some("cli-model"),
    "completion-model"
)]
#[case::cli_model(None, Some("cli-model"), Some("cli-model"), "cli-model")]
#[case::query(None, None, Some("query-model"), "query-model")]
#[case::body(None, None, None, "body-model")]
#[tokio::test]
async fn queue_prefers_the_configured_models_in_order(
    #[case] completion_model: Option<&str>,
    #[case] cli_model: Option<&str>,
    #[case] query: Option<&str>,
    #[case] expected: &str,
) {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let mut state = queue_state(&scheduler(), &completion, None);
    state.completion_model = completion_model.map(str::to_owned);
    state.cli_model = cli_model.map(str::to_owned);
    let router = queue_router(state);
    let uri = query.map_or_else(
        || "/queue/chat/completions".to_owned(),
        |model| format!("/queue/chat/completions?model={model}"),
    );
    let (status, _, _) = Box::pin(send(&router, "POST", &uri, &chat_body(1), Some(master()))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completion.seen()["model"], expected, "{uri}");
}

#[tokio::test]
async fn queue_logs_a_session_token_as_its_alias() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let fields =
        r#"{"token":"tok","is_session_token":true,"key_alias":"alias-1","user_id":"key-user"}"#;
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(key_caller(fields)),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completion.seen()["metadata"]["user_api_key"], "alias-1");
    assert_eq!(
        completion.seen()["metadata"]["user_api_key_hash"],
        "alias-1"
    );
}

#[tokio::test]
async fn queue_applies_only_truthy_cli_overrides() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let mut state = queue_state(&scheduler(), &completion, None);
    state.overrides = CliOverrides {
        temperature: Some(0.5_f64),
        request_timeout: Some(0.0_f64),
        max_tokens: Some(100),
        api_base: Some(String::new()),
    };
    let router = queue_router(state);
    let (status, _, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        r#"{"model":"m","priority":1,"messages":[],"request_timeout":30}"#,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    let seen = completion.seen();
    assert_eq!(seen["temperature"], 0.5_f64);
    assert_eq!(seen["max_tokens"], 100_i64);
    assert_eq!(seen["request_timeout"], 30_i64);
    assert!(seen.get("api_base").is_none());
}

#[tokio::test]
async fn queue_answers_a_non_streaming_completion() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object(
        r#"{"id":"resp-1","object":"chat.completion"}"#,
    ))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, headers, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(7),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-litellm-priority"], "7");
    let body = parse(&text);
    assert_eq!(body["id"], "resp-1");
    assert!(body.get("_hidden_params").is_none());
    assert!(matches!(
        completion.calls().as_slice(),
        [(data, false)] if data["priority"] == 7_i64
    ));
}

#[tokio::test]
async fn queue_reports_a_provider_failure() {
    let completion = Arc::new(FakeCompletion::new(Reply::Failure("boom".to_owned())));
    let hook = Arc::new(FakeHook::new(None));
    let hook_handle: Arc<FakeHook> = Arc::clone(&hook);
    let failure_hook: Arc<dyn QueueFailureHook> = hook_handle;
    let router = queue_router(queue_state(&scheduler(), &completion, Some(failure_hook)));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["error"]["message"],
        "Authentication Error, boom"
    );
    assert_eq!(parse(&text)["error"]["type"], "auth_error");
    assert_eq!(parse(&text)["error"]["code"], "400");
    assert_eq!(hook.call_count(), 1);
    assert_eq!(
        hook.seen(),
        vec![("Authentication Error, boom".to_owned(), "400".to_owned())]
    );
}

#[tokio::test]
async fn queue_lets_the_failure_hook_replace_the_error() {
    let completion = Arc::new(FakeCompletion::new(Reply::Failure("boom".to_owned())));
    let hook = Arc::new(FakeHook::new(Some(replacement(403))));
    let failure_hook: Arc<dyn QueueFailureHook> = hook;
    let router = queue_router(queue_state(&scheduler(), &completion, Some(failure_hook)));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(parse(&text)["error"]["message"], "replaced");
    assert_eq!(parse(&text)["error"]["code"], "403");
}

#[tokio::test]
async fn queue_refuses_a_non_object_answer_from_a_non_streaming_call() {
    let completion = Arc::new(FakeCompletion::new(Reply::Stream));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        &chat_body(1),
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["error"]["message"],
        "Authentication Error, the queued completion answered no response object"
    );
}

#[tokio::test]
async fn queue_streams_the_completion() {
    let items = vec![Ok(StreamItem::Text(r#"{"id":"c1"}"#.to_owned()))];
    let completion = Arc::new(FakeCompletion::streaming(items));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let body = r#"{"model":"m","priority":3,"messages":[],"stream":true}"#;
    let (status, headers, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/event-stream; charset=utf-8");
    assert!(headers.contains_key("cache-control"));
    assert!(headers.contains_key("x-accel-buffering"));
    assert!(text.contains(r#"data: {"id":"c1"}"#), "{text}");
    assert!(text.contains(DONE_FRAME), "{text}");
    assert!(matches!(
        completion.calls().as_slice(),
        [(data, true)] if data["stream"] == json!(true)
    ));
}

#[tokio::test]
async fn queue_reports_a_streaming_call_without_a_stream() {
    let completion = Arc::new(FakeCompletion::new(Reply::Stream));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let body = r#"{"model":"m","priority":1,"messages":[],"stream":true}"#;
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        parse(&text)["error"]["message"],
        "Authentication Error, the queued completion answered no response object"
    );
}

fn stream_failure() -> StreamFailure {
    StreamFailure {
        message: "upstream broke".to_owned(),
        error_type: FailureType::Text("upstream_error".to_owned()),
        param: FailureParam::Null,
        status_code: Some(500),
        code: None,
        body: None,
        kind: FailureKind::Other,
    }
}

#[tokio::test]
async fn queue_reports_a_stream_failure_to_the_hook() {
    let items = vec![
        Ok(StreamItem::Text(r#"{"id":"c1"}"#.to_owned())),
        Err(stream_failure()),
    ];
    let completion = Arc::new(FakeCompletion::streaming(items));
    let hook = Arc::new(FakeHook::new(None));
    let hook_handle: Arc<FakeHook> = Arc::clone(&hook);
    let failure_hook: Arc<dyn QueueFailureHook> = hook_handle;
    let router = queue_router(queue_state(&scheduler(), &completion, Some(failure_hook)));
    let body = r#"{"model":"m","priority":1,"messages":[],"stream":true}"#;
    let (status, headers, _) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/event-stream; charset=utf-8");
    assert_eq!(hook.call_count(), 1);
    assert_eq!(
        hook.seen(),
        vec![("upstream broke".to_owned(), "500".to_owned())]
    );
    assert_eq!(
        hook.data.lock().expect("the hook lock")[0]["priority"],
        1_i64
    );
}

#[tokio::test]
async fn queue_lets_the_stream_hook_replace_the_failure() {
    let items = vec![Err(stream_failure())];
    let completion = Arc::new(FakeCompletion::streaming(items));
    let hook = Arc::new(FakeHook::new(Some(replacement(429))));
    let hook_handle: Arc<FakeHook> = Arc::clone(&hook);
    let failure_hook: Arc<dyn QueueFailureHook> = hook_handle;
    let router = queue_router(queue_state(&scheduler(), &completion, Some(failure_hook)));
    let body = r#"{"model":"m","priority":1,"messages":[],"stream":true}"#;
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        body,
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hook.call_count(), 1);
    assert_eq!(
        hook.seen(),
        vec![("upstream broke".to_owned(), "500".to_owned())]
    );
    // a hook that replaces the failure ends the stream instead of writing the original frame, as the raised replacement does
    assert!(!text.contains("upstream broke"), "{text}");
}

#[tokio::test]
async fn queue_reports_a_body_it_cannot_read() {
    let completion = Arc::new(FakeCompletion::new(Reply::Object(object("{}"))));
    let router = queue_router(queue_state(&scheduler(), &completion, None));
    let (status, _, text) = Box::pin(send(
        &router,
        "POST",
        "/queue/chat/completions",
        "{",
        Some(master()),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = parse(&text)["error"]["message"].clone();
    assert!(
        message
            .as_str()
            .is_some_and(|answer| answer.starts_with("Authentication Error, ")),
        "{message}"
    );
    assert!(completion.calls().is_empty());
}
