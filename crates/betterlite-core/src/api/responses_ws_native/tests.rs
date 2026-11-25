#![cfg(test)]

mod forwarding;
mod support;

use std::sync::Arc;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::ResponsesWebSocketMessage;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::api::decisions_send::error::visible_message;
use crate::api::responses_provider_selection::provider_config::{NativeConfig, ResponsesApiConfig};
use crate::api::responses_stream_events::ProjectQuotaCallback;
use crate::api::responses_ws_frame_policy::{
    DELTA_EVENT_TYPES, InputGuardrail, OUTPUT_DONE_EVENT_TYPES,
};
use crate::api::responses_ws_native::error::WsNativeError;
use crate::api::responses_ws_native::logging::traceback_text;
use crate::api::responses_ws_native::relay::ResponsesWsRelay;
use crate::api::responses_ws_native::relay::backend::BackendError;
use crate::api::responses_ws_native::relay::capture::CaptureRelay as _;
use crate::api::responses_ws_native::relay::forward::ForwardRelay as _;
use crate::api::responses_ws_native::request::{
    build_litellm_metadata_for_ws, deployment_reasoning_default, routed_first_frame,
    ws_request_defaults,
};
use support::{
    CloseMode, EchoGuard, Entry, Event, ReceiveMode, RecordingLogging, RejectFirstQuota,
    RelayParts, ScriptedBackend, TestClient, TestError, capture_traces, json, lock, object,
    redactor, unmask_tokens,
};

fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[tokio::test]
async fn native_ws_response_create() {
    forwarding::native_ws_response_create().await;
}

fn assert_frame_rewrites() {
    let routed = Json::Text(String::from("routed"));
    assert_eq!(routed_first_frame("not-json", Some(&routed)), "not-json");
    assert_eq!(routed_first_frame("null", Some(&routed)), "null");
    assert_eq!(
        routed_first_frame(
            r#"{"type":"response.create","input":"old","other":1}"#,
            Some(&routed)
        ),
        r#"{"type":"response.create","input":"routed","other":1}"#
    );
    assert_eq!(
        routed_first_frame(
            r#"{"type":"response.create","response":{"input":"old"},"other":1}"#,
            Some(&routed)
        ),
        r#"{"type":"response.create","response":{"input":"routed"},"other":1}"#
    );
    assert_eq!(
        routed_first_frame(
            r#"{"type":"response.create","response":{"input":null}}"#,
            Some(&routed)
        ),
        r#"{"type":"response.create","response":{"input":null},"input":"routed"}"#
    );
}

fn assert_frame_noops() {
    let routed = Json::Text(String::from("routed"));
    let matching = r#"{"type":"response.create","input":"routed"}"#;
    assert_eq!(routed_first_frame(matching, Some(&routed)), matching);
    let no_routed_input = r#"{"type":"response.create","input":"old"}"#;
    assert_eq!(routed_first_frame(no_routed_input, None), no_routed_input);
    let nested_matching = r#"{"type":"response.create","response":{"input":"routed"}}"#;
    assert_eq!(
        routed_first_frame(nested_matching, Some(&routed)),
        nested_matching
    );
    let spaced_flat = r#"{ "type": "response.create", "input": "routed" }"#;
    assert_eq!(routed_first_frame(spaced_flat, Some(&routed)), spaced_flat);
    let spaced_nested =
        r#"{ "type": "response.create", "response": { "input": "routed" }, "other": 1 }"#;
    assert_eq!(
        routed_first_frame(spaced_nested, Some(&routed)),
        spaced_nested
    );
}

fn assert_reasoning_defaults() {
    let scalar = object(r#"{"reasoning_effort":"high"}"#);
    assert_eq!(
        deployment_reasoning_default(&scalar, false).unwrap(),
        Some(json(r#"{"effort":"high"}"#))
    );
    assert_eq!(
        deployment_reasoning_default(&scalar, true).unwrap(),
        Some(json(r#"{"effort":"high","summary":"detailed"}"#))
    );
    let mapping = object(r#"{"reasoning_effort":{"effort":"high"}}"#);
    assert_eq!(
        deployment_reasoning_default(&mapping, false).unwrap(),
        Some(json(r#"{"effort":"high"}"#))
    );
    let suppressed = object(r#"{"reasoning":{"effort":"none"},"reasoning_effort":"high"}"#);
    assert_eq!(
        deployment_reasoning_default(&suppressed, false).unwrap(),
        None
    );
    let null_effort = object(r#"{"reasoning_effort":null}"#);
    assert_eq!(
        deployment_reasoning_default(&null_effort, false).unwrap(),
        None
    );
    assert_eq!(
        deployment_reasoning_default(&object("{}"), false).unwrap(),
        None
    );
}

fn assert_deployment_defaults() {
    let deployment = object(
        r#"{"temperature":0.5,"reasoning_effort":"high","extra_body":{"reasoning":{"effort":"low"},"service_tier":"flex"}}"#,
    );
    let defaults = ws_request_defaults(&deployment, false).unwrap();
    assert_eq!(defaults.fill_missing.get("temperature"), Some(&json("0.5")));
    assert_eq!(
        defaults.fill_missing.get("reasoning"),
        Some(&json(r#"{"effort":"high"}"#))
    );
    assert!(!defaults.fill_missing.contains_key("extra_body"));
    assert_eq!(
        defaults.overrides.get("reasoning"),
        Some(&json(r#"{"effort":"low"}"#))
    );
    assert_eq!(
        defaults.overrides.get("service_tier"),
        Some(&json(r#""flex""#))
    );
}

fn assert_malformed_inputs() {
    assert!(matches!(
        build_litellm_metadata_for_ws(&object(r#"{"litellm_metadata":[1]}"#)),
        Err(WsNativeError::TypeError { kind: "list" })
    ));
    assert!(matches!(
        build_litellm_metadata_for_ws(&object(r#"{"metadata":["x"]}"#)),
        Err(WsNativeError::AttributeError { kind: "list" })
    ));
    assert!(matches!(
        ws_request_defaults(&object(r#"{"extra_body":[1]}"#), false),
        Err(WsNativeError::Validation { .. })
    ));
    assert!(matches!(
        build_litellm_metadata_for_ws(&object(r#"{"litellm_metadata":5}"#)),
        Err(WsNativeError::TypeError { kind: "int" })
    ));
    let zero = build_litellm_metadata_for_ws(&object(r#"{"litellm_metadata":0}"#)).unwrap();
    assert!(zero.is_empty());
    let blank = build_litellm_metadata_for_ws(&object(r#"{"litellm_metadata":""}"#)).unwrap();
    assert!(blank.is_empty());
}

async fn entry_routing_request() -> support::RecordedRequest {
    let mut entry = Entry::new();
    entry.kwargs = object(
        r#"{"input":"routed","previous_response_id":"resp_prev","temperature":0.5,"first_message":"{\"type\":\"response.create\",\"response\":{\"input\":\"old\"},\"other\":1}","other":1}"#,
    );
    entry.request().await
}

async fn non_string_first_message_request() -> support::RecordedRequest {
    let mut entry = Entry::new();
    entry.kwargs =
        object(r#"{"input":"routed","first_message":{"type":"response.create","input":"old"}}"#);
    entry.request().await
}

async fn null_routed_input_request() -> support::RecordedRequest {
    let mut entry = Entry::new();
    entry.kwargs = object(
        r#"{"input":null,"first_message":"{\"type\":\"response.create\",\"input\":\"old\"}"}"#,
    );
    entry.request().await
}

#[tokio::test]
async fn request_defaults_and_routing() {
    assert_frame_rewrites();
    assert_frame_noops();
    assert_reasoning_defaults();
    assert_deployment_defaults();
    assert_malformed_inputs();
    let request = entry_routing_request().await;
    assert_eq!(
        request.first_message.as_deref(),
        Some(r#"{"type":"response.create","response":{"input":"routed"},"other":1}"#)
    );
    assert_eq!(
        request.remaining_kwargs,
        object(r#"{"temperature":0.5,"other":1}"#)
    );
    assert_eq!(request.fill_missing.get("temperature"), Some(&json("0.5")));
    assert!(!request.fill_missing.contains_key("previous_response_id"));
    assert!(request.overrides.is_empty());
    let non_string = non_string_first_message_request().await;
    assert_eq!(non_string.first_message, None);
    let null_routed = null_routed_input_request().await;
    assert_eq!(
        null_routed.first_message.as_deref(),
        Some(r#"{"type":"response.create","input":"old"}"#)
    );
}

async fn merged_metadata_request() -> support::RecordedRequest {
    let mut entry = Entry::new();
    entry.api_base = Some(String::from("https://caller.test"));
    entry.api_key = Some(String::from("sk-call"));
    entry.resolution.api_base = Some(String::from("wss://dynamic.test"));
    entry.resolution.api_key = Some(String::from("sk-dynamic"));
    entry.kwargs = object(
        r#"{"litellm_metadata":{"trace":"t1"},"metadata":{"guardrails":["pii"]},"guardrails":["fallback"]}"#,
    );
    entry.request().await
}

async fn logged_params(kwargs: &str) -> Event {
    let mut entry = Entry::new();
    entry.kwargs = object(kwargs);
    entry.request().await;
    lock(&entry.logging.updates)[0].litellm_params.clone()
}

async fn assert_logged_metadata() {
    let caller_metadata = logged_params(r#"{"metadata":{"guardrails":["pii"]}}"#).await;
    assert_eq!(
        caller_metadata.get("metadata"),
        Some(&Json::Object(object(r#"{"guardrails":["pii"]}"#)))
    );
    let litellm_metadata = logged_params(r#"{"litellm_metadata":{"trace":"t1"}}"#).await;
    assert_eq!(
        litellm_metadata.get("metadata"),
        Some(&Json::Object(object(r#"{"trace":"t1"}"#)))
    );
    assert_eq!(
        litellm_metadata.get("litellm_metadata"),
        Some(&Json::Object(object(r#"{"trace":"t1"}"#)))
    );
    let empty_litellm_metadata = logged_params(r#"{"litellm_metadata":{}}"#).await;
    assert_eq!(empty_litellm_metadata.get("metadata"), Some(&Json::Null));
    let absent_litellm_metadata = logged_params("{}").await;
    assert_eq!(
        absent_litellm_metadata.get("litellm_metadata"),
        Some(&Json::Null)
    );
    let mut residency = Entry::new();
    residency.resolution.api_base = Some(String::from("https://eu.api.openai.com/v1"));
    residency.request().await;
    assert_eq!(
        lock(&residency.logging.updates)[0]
            .litellm_params
            .get("data_residency"),
        Some(&Json::Text(String::from("eu")))
    );
}

async fn assert_logged_credentials() {
    let mut kwargs_key = Entry::new();
    kwargs_key.kwargs = object(r#"{"api_key":"sk-kwarg"}"#);
    kwargs_key.globals.api_key = Some(SecretString::from("sk-global"));
    assert_eq!(
        kwargs_key.request().await.api_key.as_deref(),
        Some("sk-kwarg")
    );
}

async fn assert_guardrail_precedence() {
    let mut entry = Entry::new();
    entry.kwargs = object(r#"{"metadata":{"guardrails":[]},"guardrails":["fallback"]}"#);
    let fallback = entry.request().await;
    entry.kwargs = object(r#"{"metadata":{},"guardrails":[]}"#);
    let neither = entry.request().await;
    assert_eq!(
        fallback.litellm_metadata,
        object(r#"{"guardrails":["fallback"]}"#)
    );
    assert!(neither.litellm_metadata.is_empty());
}

async fn assert_credential_fallbacks() {
    let mut entry = Entry::new();
    entry.resolution.api_base = Some(String::new());
    entry.globals.api_base = Some(String::from("https://global.test"));
    entry.globals.api_key = Some(SecretString::from("sk-global"));
    entry.globals.openai_key = Some(SecretString::from("sk-openai"));
    entry.openai_api_key = Some(String::from("sk-env"));
    let global = entry.request().await;
    entry.globals.api_key = Some(SecretString::from(""));
    let openai_key = entry.request().await;
    entry.globals.openai_key = None;
    let env = entry.request().await;
    entry.openai_api_key = None;
    let none = entry.request().await;
    assert_eq!(
        vec![
            global.api_base,
            global.api_key,
            openai_key.api_key,
            env.api_key,
            none.api_key,
        ],
        vec![
            Some(String::from("https://global.test")),
            Some(String::from("sk-global")),
            Some(String::from("sk-openai")),
            Some(String::from("sk-env")),
            None,
        ]
    );
}

async fn recorded_user() -> Option<String> {
    let mut entry = Entry::new();
    entry.kwargs = object(r#"{"user":"alice"}"#);
    entry.request().await;
    lock(&entry.logging.updates)[0].user.clone()
}

async fn routing_prefix_is_stripped() -> (String, String) {
    let mut entry = Entry::new();
    entry.resolution.model = String::from("responses/gpt-4o");
    let request = entry.request().await;
    let logged = lock(&entry.logging.updates)[0].model.clone();
    (request.model, logged)
}

async fn assert_call_credentials_reach_resolution() {
    let mut entry = Entry::new();
    entry.resolution.echo_request = true;
    entry.api_base = Some(String::from("wss://caller.test"));
    entry.api_key = Some(String::from("sk-call"));
    let request = entry.request().await;
    assert_eq!(request.model, "gpt-4o");
    assert_eq!(request.api_base.as_deref(), Some("wss://caller.test"));
    assert_eq!(request.api_key.as_deref(), Some("sk-call"));
}

async fn assert_api_base_precedence() {
    let mut entry = Entry::new();
    entry.resolution.api_base = Some(String::from("wss://dynamic.test"));
    entry.kwargs = object(r#"{"api_base":"wss://kwarg.test"}"#);
    entry.globals.api_base = Some(String::from("https://global.test"));
    let dynamic = entry.request().await;
    entry.resolution.api_base = None;
    let from_kwargs = entry.request().await;
    entry.kwargs = Event::new();
    let from_globals = entry.request().await;
    entry.globals.api_base = None;
    let absent = entry.request().await;
    assert_eq!(
        vec![
            dynamic.api_base,
            from_kwargs.api_base,
            from_globals.api_base,
            absent.api_base,
        ],
        vec![
            Some(String::from("wss://dynamic.test")),
            Some(String::from("wss://kwarg.test")),
            Some(String::from("https://global.test")),
            None,
        ]
    );
}

async fn assert_provider_config() {
    let mut entry = Entry::new();
    entry.configs.native = true;
    entry.resolution.api_base = Some(String::from("wss://dynamic.test"));
    let native = entry.request().await;
    assert_eq!(
        lock(&entry.configs.seen).clone(),
        vec![(
            String::from("gpt-4o"),
            String::from("openai"),
            Some(String::from("wss://dynamic.test")),
        )]
    );
    let mut missing = Entry::new();
    missing.resolution.provider = None;
    let absent = missing.request().await;
    assert!(lock(&missing.configs.seen).is_empty());
    assert_eq!(
        native.provider_config,
        Some(ResponsesApiConfig::Native(NativeConfig {
            encrypted_agent_messages: true,
        }))
    );
    assert_eq!(absent.provider_config, None);
}

#[tokio::test]
async fn metadata_and_credentials() {
    let merged = merged_metadata_request().await;
    assert_eq!(
        merged.litellm_metadata,
        object(r#"{"trace":"t1","guardrails":["pii"]}"#)
    );
    assert_eq!(merged.api_base.as_deref(), Some("wss://dynamic.test"));
    assert_eq!(merged.api_key.as_deref(), Some("sk-dynamic"));
    assert_eq!(merged.custom_llm_provider.as_deref(), Some("openai"));
    assert_call_credentials_reach_resolution().await;
    assert_api_base_precedence().await;
    assert_guardrail_precedence().await;
    assert_credential_fallbacks().await;
    assert_provider_config().await;
    assert_logged_metadata().await;
    assert_logged_credentials().await;
}

#[tokio::test]
async fn routing_prefix_is_stripped_before_resolution() {
    let (forwarded_model, logged_model) = routing_prefix_is_stripped().await;
    assert_eq!(forwarded_model, "gpt-4o");
    assert_eq!(logged_model, "gpt-4o");
}

#[tokio::test]
async fn caller_user_reaches_logging() {
    assert_eq!(recorded_user().await.as_deref(), Some("alice"));
}

fn constructor_defaults(relay: &ResponsesWsRelay<'_, EchoGuard, TestClient, ScriptedBackend>) {
    assert!(relay.litellm_metadata.is_empty());
    assert!(relay.setup.guardrail_callbacks.is_none());
    assert!(relay.setup.output_guardrail_callbacks.is_none());
    assert!(relay.setup.quota_callbacks.is_none());
    assert!(relay.setup.authorized_model.is_none());
    assert!(relay.setup.request_defaults.is_none());
}

async fn stored_events(relay: &ResponsesWsRelay<'_, EchoGuard, TestClient, ScriptedBackend>) {
    relay
        .store_event(r#"{"type":"response.created"}"#)
        .await
        .unwrap();
    relay
        .store_event(r#"{"type":"response.completed"}"#)
        .await
        .unwrap();
    let dropped = [
        r#"{"type":"response.output_text.delta"}"#,
        r#"{"type":"unknown"}"#,
        "not-json",
    ];
    for frame in dropped {
        relay.store_event(frame).await.unwrap();
    }
    assert_eq!(relay.messages.lock().await.len(), 2);
}

async fn collected_input(
    relay: &ResponsesWsRelay<'_, EchoGuard, TestClient, ScriptedBackend>,
    logging: &RecordingLogging,
) {
    let hello = r#"{"type":"response.create","input":"hello"}"#;
    let world = r#"{"type":"response.create","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"world"}]}]}"#;
    relay.store_input(hello).await;
    relay.store_input(world).await;
    let dropped = [
        "not-json",
        r#"{"type":"response.create","input":[{"type":"message","role":"assistant","content":[{"type":"input_text","text":"no"}]}]}"#,
        r#"{"type":"response.create","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":""}]}]}"#,
        r#"{"type":"response.create","input":[{"type":"message","role":"user","content":[{"type":"input_image","text":"ignored"}]}]}"#,
        r#"{"type":"response.create","input":[{"type":"message","role":"user","content":[{"type":"input_text"}]}]}"#,
        r#"{"type":"response.create","input":[1,"text",null]}"#,
        r#"{"type":"other","input":"ignored"}"#,
    ];
    for frame in dropped {
        relay.store_input(frame).await;
    }
    let expected = vec![
        object(r#"{"role":"user","content":"hello"}"#),
        object(r#"{"role":"user","content":"world"}"#),
    ];
    assert_eq!(relay.input_messages.lock().await.clone(), expected);
    assert_eq!(lock(&logging.pre_calls).len(), 9);
    assert!(
        lock(&logging.pre_calls)
            .iter()
            .all(|(_, key)| key.is_empty())
    );
}

fn wrapped_item(frame: &str, relay_metadata: Event) -> String {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let mut request_data = Event::new();
    request_data.insert(
        String::from("litellm_metadata"),
        Json::Object(relay_metadata),
    );
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.request_data = Some(request_data);
    let relay = parts.relay();
    relay.wrap_response_event(frame).unwrap()
}

fn encrypted_metadata() -> Event {
    object(r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":"dep-1"}}"#)
}

fn assert_encrypted_wrapping() {
    let frame = r#"{"type":"response.output_item.added","item":{"encrypted_content":"cipher"}}"#;
    let wrapped = wrapped_item(frame, encrypted_metadata());
    let parsed: Event = serde_json::from_str(&wrapped).unwrap();
    let Json::Object(item) = parsed.get("item").unwrap().clone() else {
        panic!("the item stays an object");
    };
    let Some(Json::Text(content)) = item.get("encrypted_content") else {
        panic!("the encrypted content stays text");
    };
    assert_ne!(content, "cipher");
    assert!(content.ends_with(";cipher"));
    let empty_id = object(r#"{"encrypted_content_affinity_enabled":true,"model_info":{"id":""}}"#);
    assert_ne!(wrapped_item(frame, empty_id), frame);
    let disabled = object(r#"{"encrypted_content_affinity_enabled":false}"#);
    assert_eq!(wrapped_item(frame, disabled), frame);
    let missing = object(r#"{"encrypted_content_affinity_enabled":true}"#);
    assert_eq!(wrapped_item(frame, missing), frame);
    let not_item = r#"{"type":"response.output_item.added","item":"cipher"}"#;
    assert_eq!(wrapped_item(not_item, encrypted_metadata()), not_item);
    let empty = r#"{"type":"response.output_item.added","item":{"encrypted_content":""}}"#;
    assert_eq!(wrapped_item(empty, encrypted_metadata()), empty);
}

async fn unmasked_backend_frame() {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    backend
        .text(
            r#"{"type":"response.completed","response":{"output":[{"content":[{"text":"hi <EMAIL_ADDRESS_1>"}]}]}}"#,
        )
        .closing();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let guardrails = [InputGuardrail {
        callback: EchoGuard,
        unmask_pii_text: &unmask_tokens,
    }];
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.guardrails = Some(&guardrails);
    parts.request_data = Some(object(
        r#"{"metadata":{"pii_tokens":{"<EMAIL_ADDRESS_1>":"a@example.com"}}}"#,
    ));
    let relay = parts.relay();
    relay.backend_to_client().await.unwrap();
    let sent = client.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("hi a@example.com"));
}

#[tokio::test]
async fn event_capture_and_input_log() {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let mut request_data = Event::new();
    request_data.insert(String::from("litellm_metadata"), json("[1]"));
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.request_data = Some(request_data);
    let relay = parts.relay();
    constructor_defaults(&relay);
    stored_events(&relay).await;
    collected_input(&relay, &logging).await;
    assert_encrypted_wrapping();
    unmasked_backend_frame().await;
}

async fn recv_fallback_and_connection_closed() {
    let (captured, _guard) = capture_traces();
    let client = TestClient::new(Vec::new());
    let mut backend = ScriptedBackend::new();
    backend.receive = ReceiveMode::UnsupportedDecode;
    {
        let mut script = lock(&backend.recv_script);
        script.push_back(Ok(ResponsesWebSocketMessage::Text(String::from(
            r#"{"type":"response.created"}"#,
        ))));
        script.push_back(Ok(ResponsesWebSocketMessage::Bytes(
            br#"{"type":"response.completed"}"#.to_vec(),
        )));
        script.push_back(Err(BackendError::Closed(TestError(String::from(
            "the backend connection closed",
        )))));
    };
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay.backend_to_client().await.unwrap();
    assert_eq!(
        client.sent(),
        texts(&[
            r#"{"type":"response.created"}"#,
            r#"{"type":"response.completed"}"#
        ])
    );
    assert_eq!(lock(&logging.success).len(), 1);
    assert!(
        captured
            .text()
            .contains("Responses WS backend connection closed: the backend connection closed")
    );
}

async fn failed_backend_frame_is_logged() {
    let (captured, _guard) = capture_traces();
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    backend
        .text(r#"{"type":"response.created"}"#)
        .failing("boom");
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay.backend_to_client().await.unwrap();
    assert_eq!(client.sent(), texts(&[r#"{"type":"response.created"}"#]));
    assert!(
        captured
            .text()
            .contains("Error in responses WS backend_to_client: boom")
    );
    assert_eq!(lock(&logging.success).len(), 1);
}

async fn backend_send_failure_ends_forwarding() {
    let mut client = TestClient::new(Vec::new());
    client.fail_sends = true;
    let backend = ScriptedBackend::new();
    backend.text(r#"{"type":"response.created"}"#);
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay.backend_to_client().await.unwrap();
    assert_eq!(client.sent(), texts(&[r#"{"type":"response.created"}"#]));
}

async fn output_events_are_dropped() {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    for kind in DELTA_EVENT_TYPES
        .iter()
        .chain(OUTPUT_DONE_EVENT_TYPES.iter())
    {
        backend.text(&format!(r#"{{"type":"{kind}"}}"#));
    }
    backend.text("not-json").closing();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let outputs = [EchoGuard];
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.outputs = Some(outputs.as_slice());
    let relay = parts.relay();
    relay.backend_to_client().await.unwrap();
    assert_eq!(client.sent(), texts(&["not-json"]));
}

async fn array_frame_fails_wrapping() {
    let (captured, _guard) = capture_traces();
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    backend.text("[1,2,3]").closing();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay.backend_to_client().await.unwrap();
    assert!(client.sent().is_empty());
    assert!(
        captured.text().contains(
            "Error in responses WS backend_to_client: 'list' object has no attribute 'get'"
        )
    );
}

async fn invalid_utf8_sends_nothing() {
    let (captured, _guard) = capture_traces();
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    backend.bytes(&[0xff, 0xfe]).closing();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay.backend_to_client().await.unwrap();
    assert!(client.sent().is_empty());
    assert!(
        captured
            .text()
            .contains("Error in responses WS backend_to_client: invalid utf-8")
    );
}

async fn client_termination_closes_backend() {
    let (captured, _guard) = capture_traces();
    let mut client = TestClient::new(Vec::new());
    client.yield_first = true;
    let mut backend = ScriptedBackend::new();
    backend.receive = ReceiveMode::Blocked;
    backend.close = CloseMode::Fails;
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.authorized_model = Some("gpt-4o");
    parts.provider = Some("openai");
    let relay = parts.relay();
    relay.messages.lock().await.push(object(
        r#"{"type":"response.failed","response":{"error":{"type":"server_error","message":"upstream broke","code":"E_UP"}}}"#,
    ));
    let failure = relay
        .bidirectional_forward()
        .await
        .expect("the stored failure comes back");
    assert_eq!(failure.metadata().status_code, 500);
    assert!(logging.effects().contains(&String::from("failure")));
    assert!(
        captured
            .text()
            .contains("Responses WS client_to_backend ended")
    );
}

#[tokio::test]
async fn backend_close_and_disconnect() {
    recv_fallback_and_connection_closed().await;
    failed_backend_frame_is_logged().await;
    backend_send_failure_ends_forwarding().await;
    output_events_are_dropped().await;
    array_frame_fails_wrapping().await;
    invalid_utf8_sends_nothing().await;
    client_termination_closes_backend().await;
}

async fn rejected_frame_effects() {
    let client = TestClient::new(vec![
        r#"{"type":"response.create","input":"first"}"#,
        r#"{"type":"response.create","input":"second"}"#,
    ]);
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let quota: Arc<dyn ProjectQuotaCallback> = Arc::new(RejectFirstQuota::new("quota full"));
    let quotas = [quota];
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.quota = Some(quotas.as_slice());
    let relay = parts.relay();
    relay.client_to_backend().await;
    assert_eq!(
        client.sent(),
        texts(&[
            r#"{"type":"error","error":{"type":"rate_limit_exceeded","message":"quota full"}}"#
        ])
    );
    assert_eq!(
        backend.sent(),
        texts(&[r#"{"type":"response.create","input":"second"}"#])
    );
    let pre_calls = lock(&logging.pre_calls).clone();
    assert_eq!(
        pre_calls,
        vec![(
            String::from(r#"{"type":"response.create","input":"second"}"#),
            String::new(),
        )]
    );
    assert!(relay.messages.lock().await.is_empty());
}

async fn rejected_send_failure_is_swallowed() {
    let mut client = TestClient::new(vec![r#"{"type":"response.create","input":"first"}"#]);
    client.fail_sends = true;
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let quota: Arc<dyn ProjectQuotaCallback> = Arc::new(RejectFirstQuota::new("quota full"));
    let quotas = [quota];
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.quota = Some(quotas.as_slice());
    let relay = parts.relay();
    relay.client_to_backend().await;
    assert_eq!(client.sent().len(), 1);
    assert!(backend.sent().is_empty());
    assert!(relay.messages.lock().await.is_empty());
}

#[tokio::test]
async fn rate_limit_drop() {
    rejected_frame_effects().await;
    rejected_send_failure_is_swallowed().await;
}

async fn logged_failure() -> (RecordingLogging, ErrorClass) {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let mut parts = RelayParts::standard(&client, &backend, &logging, &redactor);
    parts.authorized_model = Some("gpt-4o");
    parts.provider = Some("openai");
    let relay = parts.relay();
    relay
        .store_input(r#"{"type":"response.create","input":"hello"}"#)
        .await;
    relay
        .store_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7},"service_tier":"flex"}}"#,
        )
        .await
        .unwrap();
    relay
        .store_event(
            r#"{"type":"response.failed","response":{"error":{"type":"server_error","message":"upstream broke","code":"E_UP"},"service_tier":"flex"}}"#,
        )
        .await
        .unwrap();
    relay.log_messages().await.unwrap();
    let failure = lock(&logging.failures)[0].clone();
    (logging, failure)
}

fn assert_failure_usage(logging: &RecordingLogging) {
    assert_eq!(
        logging.effects(),
        texts(&[
            "pre_call",
            "input_messages",
            "cost",
            "partial_usage",
            "failure"
        ])
    );
    assert_eq!(lock(&logging.inputs).len(), 1);
    let partial = lock(&logging.partial).clone();
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].0.prompt_tokens, 3);
    assert_eq!(partial[0].0.completion_tokens, 4);
    assert_eq!(partial[0].0.total_tokens, 7);
    assert_eq!(partial[0].1, Decimal::ZERO);
    assert_eq!(
        lock(&logging.charged).clone(),
        vec![(Some(String::from("flex")), None)]
    );
    assert_eq!(lock(&logging.failure_prefers_async).clone(), vec![true]);
}

fn assert_failure_mapping(logging: &RecordingLogging, failure: &ErrorClass) {
    let metadata = failure.metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(
        metadata.error_type,
        WireField::Text(String::from("internal_server_error"))
    );
    assert_eq!(metadata.param, WireField::Null);
    let visible = visible_message(failure);
    assert_eq!(
        visible,
        "litellm.InternalServerError: InternalServerError: OpenAIException - upstream broke"
    );
    assert_eq!(
        lock(&logging.tracebacks)[0],
        format!("Traceback (most recent call last):\n{visible}")
    );
    assert_eq!(lock(&logging.tracebacks)[0], traceback_text(failure));
}

async fn failure_is_mapped_and_billed() {
    let (logging, failure) = logged_failure().await;
    assert_failure_usage(&logging);
    assert_failure_mapping(&logging, &failure);
}

async fn success_dispatches_only() {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay
        .store_event(r#"{"type":"response.created"}"#)
        .await
        .unwrap();
    relay
        .store_event(r#"{"type":"response.completed"}"#)
        .await
        .unwrap();
    relay.log_messages().await.unwrap();
    assert_eq!(logging.effects(), texts(&["success"]));
    assert_eq!(lock(&logging.success_prefers_async).clone(), vec![true]);
    assert!(lock(&logging.partial).is_empty());
}

async fn incomplete_and_unknown() {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay
        .store_event(r#"{"type":"response.incomplete"}"#)
        .await
        .unwrap();
    relay.store_event(r#"{"type":"unknown"}"#).await.unwrap();
    relay.log_messages().await.unwrap();
    assert_eq!(logging.effects(), texts(&["success"]));
    assert_eq!(lock(&logging.success)[0].len(), 1);
    assert!(lock(&logging.partial).is_empty());
}

async fn empty_messages() {
    let client = TestClient::new(Vec::new());
    let backend = ScriptedBackend::new();
    let logging = RecordingLogging::default();
    let redactor = redactor();
    let relay = RelayParts::standard(&client, &backend, &logging, &redactor).relay();
    relay.log_messages().await.unwrap();
    assert!(logging.effects().is_empty());
}

#[tokio::test]
async fn failure_usage_logging() {
    failure_is_mapped_and_billed().await;
    success_dispatches_only().await;
    incomplete_and_unknown().await;
    empty_messages().await;
}
