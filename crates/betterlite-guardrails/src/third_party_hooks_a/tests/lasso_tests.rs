use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::lasso_classification::Configured;
use rstest::rstest;
use secrecy::SecretString;
use tokio::net::TcpListener;
use ulid::Ulid;

use super::support::{Call, captured_body, chat_data, list, message, object, serve, text};
use crate::lasso::{ConversationCache, LassoGuardrail};
use crate::lasso_masking::wire::ModelResponseShape;
use crate::third_party_hooks_a::error::ThirdPartyHookError;
use crate::third_party_hooks_a::lasso::{self, LassoRequestHook, PostCallResponse};

#[derive(Default)]
struct TestCache(Mutex<BTreeMap<String, String>>);

impl ConversationCache for TestCache {
    type Error = fmt::Error;

    fn get_cache(&self, key: &str) -> Result<Option<String>, fmt::Error> {
        let entries = self.0.lock().map_err(|_poisoned| fmt::Error)?;
        Ok(entries.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: &str, _ttl: Duration) -> Result<(), fmt::Error> {
        let mut entries = self.0.lock().map_err(|_poisoned| fmt::Error)?;
        entries.insert(key.to_owned(), value.to_owned());
        drop(entries);
        Ok(())
    }
}

fn lasso_guardrail(base: &str, mask: bool) -> LassoGuardrail {
    LassoGuardrail {
        settings: Configured {
            guardrail_name: "lasso".to_owned(),
            api_key: SecretString::from("lasso-key"),
            api_base: base.to_owned(),
            user_id: None,
            conversation_id: None,
            mask,
        },
        client: reqwest::Client::default(),
        timeout: Duration::from_secs(5),
        new_id: Ulid::generate,
    }
}

fn allowed_answer() -> String {
    r#"{"violations_detected":false,"deputies":{},"findings":{}}"#.to_owned()
}

fn block_answer() -> String {
    r#"{"violations_detected":true,"deputies":{"toxicity":true},"findings":{"toxicity":[{"action":"BLOCK"}]}}"#.to_owned()
}

fn masking_answer(messages: &str) -> String {
    format!(
        r#"{{"violations_detected":true,"deputies":{{"pii":true}},"findings":{{"pii":[{{"action":"AUTO_MASKING"}}]}},"messages":{messages}}}"#
    )
}

fn body_of(call: &Call) -> BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = captured_body(std::slice::from_ref(call), 0) else {
        panic!("the payload is a JSON object");
    };
    fields
}

fn sent_messages(body: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Array(messages)) = body.get("messages") else {
        panic!("the payload has a messages array");
    };
    messages.clone()
}

fn header(call: &Call, name: &str) -> String {
    call.headers.get(name).cloned().unwrap_or_default()
}

fn response_of(raw: &str) -> ModelResponseShape {
    serde_json::from_str(raw).expect("response")
}

#[tokio::test]
async fn lasso_pre_call_classifies_the_expanded_messages() {
    let (base, _server, captured) =
        serve("/classify", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut data = chat_data(&[message("system", "be nice"), message("user", "hello")]);
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("allowed");
    assert_eq!(
        data,
        chat_data(&[message("system", "be nice"), message("user", "hello")])
    );
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classify");
    assert_eq!(header(&calls[0], "lasso-api-key"), "lasso-key");
    assert!(!header(&calls[0], "lasso-conversation-id").is_empty());
    let body = body_of(&calls[0]);
    assert_eq!(body.get("messageType"), Some(&text("PROMPT")));
    assert_eq!(
        body.get("source"),
        Some(&object(&[("type", text("litellm"))]))
    );
    assert_eq!(
        body.get("sessionId"),
        Some(&text(&header(&calls[0], "lasso-conversation-id")))
    );
    assert!(!body.contains_key("userId"));
    assert!(!body.contains_key("tools"));
    assert_eq!(
        sent_messages(&body),
        vec![message("system", "be nice"), message("user", "hello")]
    );
}

#[tokio::test]
async fn lasso_gate_off_returns_the_data_unchanged() {
    let (base, _server, captured) = serve("/classify", Vec::new()).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut data = chat_data(&[message("user", "hello")]);
    let original = data.clone();
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        false,
    )
    .await
    .expect("skipped");
    assert_eq!(data, original);
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lasso_moderation_runs_the_prompt_check() {
    let (base, _server, captured) =
        serve("/classify", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut data = chat_data(&[message("user", "hello")]);
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::Moderation,
        true,
    )
    .await
    .expect("allowed");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classify");
    assert_eq!(body_of(&calls[0]).get("messageType"), Some(&text("PROMPT")));
}

async fn pre_call_hello(guardrail: &LassoGuardrail, cache: &TestCache) {
    let mut data = chat_data(&[message("user", "hello")]);
    lasso::hook(guardrail, &mut data, cache, LassoRequestHook::PreCall, true)
        .await
        .expect("allowed");
}

#[tokio::test]
async fn lasso_pre_call_reuses_one_conversation_id_for_a_call() {
    let (base, _server, captured) = serve(
        "/classify",
        vec![
            (StatusCode::OK, allowed_answer()),
            (StatusCode::OK, allowed_answer()),
        ],
    )
    .await;
    let guardrail = lasso_guardrail(&base, false);
    let cache = TestCache::default();
    pre_call_hello(&guardrail, &cache).await;
    pre_call_hello(&guardrail, &cache).await;
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 2);
    assert_eq!(
        header(&calls[0], "lasso-conversation-id"),
        header(&calls[1], "lasso-conversation-id")
    );
}

#[tokio::test]
async fn lasso_classifix_writes_the_masked_text_back() {
    let answer = masking_answer(
        r#"[{"role":"system","content":"be nice"},{"role":"user","content":"my name is <REDACTED>"}]"#,
    );
    let (base, _server, captured) = serve("/classifix", vec![(StatusCode::OK, answer)]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = chat_data(&[
        message("system", "be nice"),
        message("user", "my name is Max"),
    ]);
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("masked");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classifix");
    assert_eq!(
        sent_messages(&body_of(&calls[0])),
        vec![
            message("system", "be nice"),
            message("user", "my name is Max")
        ]
    );
    assert_eq!(
        data.get("messages"),
        Some(&list(&[
            message("system", "be nice"),
            message("user", "my name is <REDACTED>")
        ]))
    );
    assert!(!data.contains_key("input"));
}

#[tokio::test]
async fn lasso_classifix_keeps_the_request_without_violations() {
    let answer = r#"{"violations_detected":false,"messages":[{"role":"user","content":"other"}]}"#;
    let (base, _server, _captured) =
        serve("/classifix", vec![(StatusCode::OK, answer.to_owned())]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = chat_data(&[message("user", "hello")]);
    let original = data.clone();
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("allowed");
    assert_eq!(data, original);
}

#[tokio::test]
async fn lasso_classifix_blocks_a_block_finding() {
    let (base, _server, _captured) =
        serve("/classifix", vec![(StatusCode::OK, block_answer())]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = chat_data(&[message("user", "hello")]);
    let error = lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    let ThirdPartyHookError::Rejection { detail, .. } = &error else {
        panic!("a blocked Lasso verdict is a rejection, got {error}");
    };
    let GuardrailValue::Object(detail) = detail else {
        panic!("the detail is a JSON object");
    };
    assert_eq!(
        detail.get("error"),
        Some(&text("Violated Lasso guardrail policy"))
    );
    assert_eq!(
        detail.get("detection_message"),
        Some(&text("Blocking violations detected: toxicity"))
    );
    let Some(GuardrailValue::Object(response)) = detail.get("lasso_response") else {
        panic!("the detail carries the Lasso response");
    };
    assert_eq!(
        response.get("violations_detected"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn lasso_input_list_forces_the_classify_endpoint() {
    let (base, _server, captured) =
        serve("/classify", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = BTreeMap::from([
        ("litellm_call_id".to_owned(), text("call-1")),
        (
            "input".to_owned(),
            list(&[text("first document"), text("second document")]),
        ),
    ]);
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("allowed");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classify");
    assert_eq!(
        sent_messages(&body_of(&calls[0])),
        vec![
            message("user", "first document"),
            message("user", "second document")
        ]
    );
}

#[tokio::test]
async fn lasso_text_input_is_inspected_and_masked_back() {
    let answer = masking_answer(r#"[{"role":"user","content":"my name is <REDACTED>"}]"#);
    let (base, _server, captured) = serve("/classifix", vec![(StatusCode::OK, answer)]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = BTreeMap::from([
        ("litellm_call_id".to_owned(), text("call-1")),
        ("input".to_owned(), text("my name is Max")),
    ]);
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("masked");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classifix");
    assert_eq!(
        sent_messages(&body_of(&calls[0])),
        vec![message("user", "my name is Max")]
    );
    assert_eq!(data.get("input"), Some(&text("my name is <REDACTED>")));
    assert!(!data.contains_key("messages"));
}

#[tokio::test]
async fn lasso_multimodal_content_falls_back_to_classify() {
    let (base, _server, captured) =
        serve("/classify", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = chat_data(&[object(&[
        ("role", text("user")),
        (
            "content",
            list(&[object(&[("type", text("text")), ("text", text("look"))])]),
        ),
    ])]);
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("allowed");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classify");
    assert_eq!(
        sent_messages(&body_of(&calls[0])),
        vec![message("user", "look")]
    );
}

#[tokio::test]
async fn lasso_nothing_to_inspect_returns_without_a_call() {
    let (base, _server, captured) = serve("/classify", Vec::new()).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut data = BTreeMap::from([("messages".to_owned(), list(&[]))]);
    let original = data.clone();
    lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect("skipped");
    assert_eq!(data, original);
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lasso_messages_of_the_wrong_type_fail_with_the_field_name() {
    let (base, _server, _captured) = serve("/classify", Vec::new()).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut data = BTreeMap::from([("messages".to_owned(), text("hello"))]);
    let error = lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect_err("wrong type");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Failure { message } if message.contains("messages")
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn lasso_timeout_is_a_plain_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let mut guardrail = lasso_guardrail(&base, false);
    guardrail.timeout = Duration::from_millis(200);
    let mut data = chat_data(&[message("user", "hello")]);
    let error = lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect_err("timeout");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Failure { message } if message == "Lasso API timeout"
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    drop(listener);
}

#[rstest]
#[case(StatusCode::UNAUTHORIZED, "Invalid API key")]
#[case(StatusCode::TOO_MANY_REQUESTS, "Lasso API rate limit exceeded")]
#[case(StatusCode::INTERNAL_SERVER_ERROR, "API error: 500")]
#[tokio::test]
async fn lasso_status_errors_use_litellm_texts(#[case] status: StatusCode, #[case] expected: &str) {
    let (base, _server, _captured) = serve("/classify", vec![(status, "nope".to_owned())]).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut data = chat_data(&[message("user", "hello")]);
    let error = lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect_err("status error");
    assert!(
        matches!(&error, ThirdPartyHookError::Failure { message } if message == expected),
        "{status} answered {error}"
    );
}

#[tokio::test]
async fn lasso_malformed_answer_is_wrapped() {
    let (base, _server, _captured) =
        serve("/classify", vec![(StatusCode::OK, "not json".to_owned())]).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut data = chat_data(&[message("user", "hello")]);
    let error = lasso::hook(
        &guardrail,
        &mut data,
        &TestCache::default(),
        LassoRequestHook::PreCall,
        true,
    )
    .await
    .expect_err("malformed answer");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Failure { message }
            if message.starts_with("Failed to verify request safety with Lasso API: ")
    ));
}

#[tokio::test]
async fn lasso_post_call_classifies_the_assistant_turn() {
    let (base, _server, captured) =
        serve("/classify", vec![(StatusCode::OK, allowed_answer())]).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut response = response_of(
        r#"{"choices":[{"message":{"role":"assistant","content":"hello","tool_calls":[{"id":"call_1","function":{"name":"lookup","arguments":"{\"city\":\"Paris\"}"}}]}}]}"#,
    );
    lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        true,
    )
    .await
    .expect("allowed");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classify");
    let body = body_of(&calls[0]);
    assert_eq!(body.get("messageType"), Some(&text("COMPLETION")));
    assert_eq!(
        body.get("sessionId"),
        Some(&text(&header(&calls[0], "lasso-conversation-id")))
    );
    assert_eq!(
        sent_messages(&body),
        vec![
            message("assistant", "hello"),
            object(&[
                ("role", text("model")),
                (
                    "content",
                    object(&[
                        ("type", text("tool_use")),
                        ("id", text("call_1")),
                        ("name", text("lookup")),
                        ("input", object(&[("city", text("Paris"))])),
                    ]),
                ),
            ]),
        ]
    );
}

#[tokio::test]
async fn lasso_post_call_blocks_with_litellm_detail() {
    let (base, _server, _captured) =
        serve("/classify", vec![(StatusCode::OK, block_answer())]).await;
    let guardrail = lasso_guardrail(&base, false);
    let mut response =
        response_of(r#"{"choices":[{"message":{"role":"assistant","content":"rude"}}]}"#);
    let error = lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        true,
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert!(matches!(
        &error,
        ThirdPartyHookError::Rejection { detail, .. }
            if *detail == object(&[
                ("error", text("Violated Lasso guardrail policy")),
                (
                    "detection_message",
                    text("Blocking violations detected: toxicity")
                ),
                (
                    "lasso_response",
                    object(&[
                        ("violations_detected", GuardrailValue::Boolean(true)),
                        ("deputies", object(&[("toxicity", GuardrailValue::Boolean(true))])),
                        (
                            "findings",
                            object(&[(
                                "toxicity",
                                list(&[object(&[("action", text("BLOCK"))])])
                            )])
                        ),
                    ])
                ),
            ])
    ));
    assert_eq!(
        response.choices[0]
            .message
            .as_ref()
            .expect("message")
            .content
            .as_deref(),
        Some("rude")
    );
}

#[tokio::test]
async fn lasso_post_call_masks_the_response_text() {
    let answer = masking_answer(r#"[{"role":"assistant","content":"call me <REDACTED>"}]"#);
    let (base, _server, captured) = serve("/classifix", vec![(StatusCode::OK, answer)]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut response =
        response_of(r#"{"choices":[{"message":{"role":"assistant","content":"call me Max"}}]}"#);
    lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        true,
    )
    .await
    .expect("masked");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/classifix");
    assert_eq!(
        response.choices[0]
            .message
            .as_ref()
            .expect("message")
            .content
            .as_deref(),
        Some("call me <REDACTED>")
    );
}

#[tokio::test]
async fn lasso_post_call_without_messages_does_not_call() {
    let (base, _server, captured) = serve("/classify", Vec::new()).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut response = response_of(r#"{"choices":[{"message":null},{"finish_reason":"stop"}]}"#);
    lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        true,
    )
    .await
    .expect("skipped");
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lasso_post_call_ignores_an_unexpected_response() {
    let (base, _server, captured) = serve("/classify", Vec::new()).await;
    let guardrail = lasso_guardrail(&base, true);
    lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::Unexpected,
        &TestCache::default(),
        true,
    )
    .await
    .expect("skipped");
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lasso_post_call_skips_a_gated_hook() {
    let (base, _server, captured) = serve("/classify", Vec::new()).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut response =
        response_of(r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#);
    lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        false,
    )
    .await
    .expect("skipped");
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn lasso_post_call_mask_failure_wraps_the_transport_error() {
    let (base, _server, _captured) = serve(
        "/classifix",
        vec![(StatusCode::INTERNAL_SERVER_ERROR, "nope".to_owned())],
    )
    .await;
    let guardrail = lasso_guardrail(&base, true);
    let mut response =
        response_of(r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#);
    let error = lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        true,
    )
    .await
    .expect_err("failed");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Failure { message }
            if message == "Failed to apply post-call masking: API error: 500"
    ));
}

#[tokio::test]
async fn lasso_post_call_mask_rejection_passes_unwrapped() {
    let (base, _server, _captured) =
        serve("/classifix", vec![(StatusCode::OK, block_answer())]).await;
    let guardrail = lasso_guardrail(&base, true);
    let mut response =
        response_of(r#"{"choices":[{"message":{"role":"assistant","content":"rude"}}]}"#);
    let error = lasso::post_call_success(
        &guardrail,
        &BTreeMap::new(),
        PostCallResponse::ChatCompletion(&mut response),
        &TestCache::default(),
        true,
    )
    .await
    .expect_err("blocked");
    assert!(matches!(error, ThirdPartyHookError::Rejection { .. }));
}
