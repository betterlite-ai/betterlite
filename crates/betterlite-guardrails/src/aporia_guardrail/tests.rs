#![cfg(test)]

use std::any::TypeId;
use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::HeaderMap as AxumHeaderMap;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName};
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::task::JoinSet;
use tracing::subscriber::with_default;

use super::{
    AporiaGuardrail, AporiaGuardrailConfigModel, AporiaGuardrailError, EventHook, ModerationGate,
    SUPPORTED_EVENT_HOOKS, Settings, ValidationTarget, prepare_aporia_request,
};
use crate::generic_guardrail_contracts::{GuardrailExtraValue, GuardrailMessage};

fn message(role: &str, content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailExtraValue::Text(content.to_owned()),
        )]),
    }
}

fn settings(api_base: &str) -> Settings {
    Settings {
        api_key: SecretString::from("secret".to_owned()),
        api_base: api_base.to_owned(),
    }
}

fn guardrail(api_base: &str) -> AporiaGuardrail {
    AporiaGuardrail {
        settings: settings(api_base),
        client: reqwest::Client::default(),
        timeout: Duration::from_secs(30),
    }
}

#[test]
fn aporia_supported_event_hooks_are_during_call_and_post_call() {
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).unwrap(),
        r#"["during_call","post_call"]"#
    );
    assert_eq!(SUPPORTED_EVENT_HOOKS[0], EventHook::DuringCall);
    assert_eq!(SUPPORTED_EVENT_HOOKS[1], EventHook::PostCall);
}

#[test]
fn aporia_settings_resolve_prefers_constructor_arguments_over_environment() {
    let lookup = BTreeMap::from([
        ("APORIO_API_KEY".to_owned(), "env-key".to_owned()),
        ("APORIO_API_BASE".to_owned(), "env-base".to_owned()),
    ]);
    let resolved = Settings::resolve(
        Some("explicit-key".to_owned()),
        Some("explicit-base".to_owned()),
        &lookup,
    )
    .expect("both values provided");
    assert_eq!(resolved.api_key.expose_secret(), "explicit-key");
    assert_eq!(resolved.api_base, "explicit-base");
}

#[test]
fn aporia_settings_resolve_falls_back_to_aporio_environment_variables() {
    let lookup = BTreeMap::from([
        ("APORIO_API_KEY".to_owned(), "env-key".to_owned()),
        ("APORIO_API_BASE".to_owned(), "env-base".to_owned()),
    ]);
    let resolved = Settings::resolve(None, None, &lookup).expect("environment provides both");
    assert_eq!(resolved.api_key.expose_secret(), "env-key");
    assert_eq!(resolved.api_base, "env-base");
}

#[test]
fn aporia_settings_resolve_errors_when_the_api_key_is_missing_from_both_sources() {
    let lookup: BTreeMap<String, String> =
        BTreeMap::from([("APORIO_API_BASE".to_owned(), "env-base".to_owned())]);
    let error = Settings::resolve(None, None, &lookup).expect_err("api key is missing");
    assert!(matches!(
        error,
        AporiaGuardrailError::MissingEnvironment {
            name: "APORIO_API_KEY"
        }
    ));
}

#[test]
fn aporia_transform_messages_keeps_supported_roles_untouched() {
    let messages = vec![
        message("system", "a"),
        message("user", "b"),
        message("assistant", "c"),
    ];
    let transformed = AporiaGuardrail::transform_messages(messages.clone());
    assert_eq!(transformed, messages);
}

#[test]
fn aporia_transform_messages_replaces_unsupported_roles_with_other_and_preserves_fields() {
    let mut tool_message = message("tool", "result");
    tool_message.extra.insert(
        "tool_call_id".to_owned(),
        GuardrailExtraValue::Text("1".to_owned()),
    );
    let transformed = AporiaGuardrail::transform_messages(vec![tool_message.clone()]);
    assert_eq!(transformed.len(), 1);
    assert_eq!(transformed[0].role, "other");
    assert_eq!(transformed[0].extra, tool_message.extra);
}

#[test]
fn aporia_prepare_request_sets_validation_target_both_when_messages_and_response_present() {
    let request = prepare_aporia_request(
        vec![message("user", "hi")],
        Some("ok".to_owned()),
        BTreeMap::new(),
    );
    assert_eq!(request.validation_target, Some(ValidationTarget::Both));
    assert_eq!(request.response, Some("ok".to_owned()));
}

#[test]
fn aporia_prepare_request_sets_validation_target_prompt_when_only_messages_present() {
    let request = prepare_aporia_request(vec![message("user", "hi")], None, BTreeMap::new());
    assert_eq!(request.validation_target, Some(ValidationTarget::Prompt));
}

#[test]
fn aporia_prepare_request_sets_validation_target_response_when_only_response_present() {
    let request = prepare_aporia_request(Vec::new(), Some("ok".to_owned()), BTreeMap::new());
    assert_eq!(request.validation_target, Some(ValidationTarget::Response));
}

#[test]
fn aporia_prepare_request_omits_validation_target_when_neither_is_present() {
    let request = prepare_aporia_request(Vec::new(), None, BTreeMap::new());
    assert_eq!(request.validation_target, None);
}

#[derive(Deserialize)]
struct Captured {
    messages: Vec<GuardrailMessage>,
    #[serde(default)]
    response: Option<String>,
}

#[derive(Deserialize)]
struct CapturedDynamicParams {
    #[serde(default)]
    project_id: Option<String>,
}

async fn call_with_mock_server(
    status: StatusCode,
    response_body: String,
) -> (Result<(), AporiaGuardrailError>, HeaderMap, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let headers = Arc::new(Mutex::new(HeaderMap::new()));
    let body = Arc::new(Mutex::new(Vec::new()));
    let headers_for_handler = Arc::clone(&headers);
    let body_for_handler = Arc::clone(&body);
    let answer = move |request_headers: AxumHeaderMap, bytes: Bytes| {
        let headers_for_body = Arc::clone(&headers_for_handler);
        let body_for_body = Arc::clone(&body_for_handler);
        async move {
            *headers_for_body.lock().unwrap() = request_headers;
            *body_for_body.lock().unwrap() = bytes.to_vec();
            (status, response_body)
        }
    };
    let router = Router::new().route("/validate", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let instance = guardrail(&base);
    let messages = vec![message("user", "hello")];
    let result = instance
        .make_aporia_api_request(messages, None, &BTreeMap::new())
        .await;
    let captured_headers = headers.lock().unwrap().clone();
    let captured_body = body.lock().unwrap().clone();
    let captured: Captured = serde_json::from_slice(&captured_body).unwrap();
    (result, captured_headers, captured)
}

#[tokio::test]
async fn aporia_make_api_request_sends_the_x_aporia_api_key_header_and_json_content_type() {
    let (result, headers, _captured) =
        call_with_mock_server(StatusCode::OK, r#"{"action": "pass"}"#.to_owned()).await;
    result.expect("a pass action does not error");
    assert_eq!(
        headers
            .get(HeaderName::from_static("x-aporia-api-key"))
            .unwrap()
            .to_str()
            .unwrap(),
        "secret"
    );
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn aporia_make_api_request_merges_dynamic_params_into_the_json_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body = Arc::new(Mutex::new(Vec::new()));
    let body_for_handler = Arc::clone(&body);
    let answer = move |bytes: Bytes| {
        let body_for_body = Arc::clone(&body_for_handler);
        async move {
            *body_for_body.lock().unwrap() = bytes.to_vec();
            (StatusCode::OK, r#"{"action": "pass"}"#.to_owned())
        }
    };
    let router = Router::new().route("/validate", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let instance = guardrail(&base);
    let dynamic_params = BTreeMap::from([(
        "project_id".to_owned(),
        GuardrailValue::Text("p-1".to_owned()),
    )]);
    instance
        .make_aporia_api_request(vec![message("user", "hi")], None, &dynamic_params)
        .await
        .expect("pass action does not error");
    let captured: CapturedDynamicParams =
        serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.project_id, Some("p-1".to_owned()));
}

#[tokio::test]
async fn aporia_make_api_request_blocks_when_action_is_block() {
    let (result, _headers, _captured) = call_with_mock_server(
        StatusCode::OK,
        r#"{"action": "block", "validation": "unsafe"}"#.to_owned(),
    )
    .await;
    let error = result.expect_err("a block action raises");
    let AporiaGuardrailError::Blocked { response } = error else {
        panic!("expected a Blocked error");
    };
    assert_eq!(response.action, Some("block".to_owned()));
}

#[tokio::test]
async fn aporia_make_api_request_rejects_a_non_success_status() {
    let (result, _headers, _captured) =
        call_with_mock_server(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    assert!(matches!(
        result,
        Err(AporiaGuardrailError::HttpStatus { status })
            if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[tokio::test]
async fn aporia_post_call_hook_skips_without_calling_the_api_when_should_run_is_false() {
    let instance = guardrail("http://127.0.0.1:1");
    let ran = instance
        .async_post_call_success_hook(
            false,
            Some("reply".to_owned()),
            Vec::new(),
            &BTreeMap::new(),
        )
        .await
        .expect("skipped guardrails do not error");
    assert!(!ran);
}

#[tokio::test]
async fn aporia_post_call_hook_skips_without_calling_the_api_when_response_string_is_none() {
    let instance = guardrail("http://127.0.0.1:1");
    let ran = instance
        .async_post_call_success_hook(true, None, Vec::new(), &BTreeMap::new())
        .await
        .expect("a missing response string skips the call");
    assert!(!ran);
}

#[tokio::test]
async fn aporia_post_call_hook_sends_the_raw_untransformed_messages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body = Arc::new(Mutex::new(Vec::new()));
    let body_for_handler = Arc::clone(&body);
    let answer = move |bytes: Bytes| {
        let body_for_body = Arc::clone(&body_for_handler);
        async move {
            *body_for_body.lock().unwrap() = bytes.to_vec();
            (StatusCode::OK, r#"{"action": "pass"}"#.to_owned())
        }
    };
    let router = Router::new().route("/validate", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let instance = guardrail(&base);
    let raw_messages = vec![message("tool", "untouched")];
    let ran = instance
        .async_post_call_success_hook(
            true,
            Some("reply".to_owned()),
            raw_messages,
            &BTreeMap::new(),
        )
        .await
        .expect("the api call succeeds");
    assert!(ran);
    let captured: Captured = serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.messages[0].role, "tool");
    assert_eq!(captured.response, Some("reply".to_owned()));
}

#[tokio::test]
async fn aporia_moderation_hook_skips_without_calling_the_api_when_should_run_is_false() {
    let instance = guardrail("http://127.0.0.1:1");
    let ran = instance
        .async_moderation_hook(
            ModerationGate {
                should_run: false,
                should_proceed: true,
            },
            Some(vec![message("user", "hi")]),
            &BTreeMap::new(),
        )
        .await
        .expect("skipped guardrails do not error");
    assert!(!ran);
}

#[tokio::test]
async fn aporia_moderation_hook_skips_without_calling_the_api_when_should_proceed_is_false() {
    let instance = guardrail("http://127.0.0.1:1");
    let ran = instance
        .async_moderation_hook(
            ModerationGate {
                should_run: true,
                should_proceed: false,
            },
            Some(vec![message("user", "hi")]),
            &BTreeMap::new(),
        )
        .await
        .expect("a metadata-based skip does not error");
    assert!(!ran);
}

#[derive(Clone, Default)]
struct Captured0(Arc<Mutex<Vec<u8>>>);

impl Write for Captured0 {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn aporia_moderation_hook_warns_and_skips_when_no_messages_are_present() {
    let captured = Captured0::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let instance = guardrail("http://127.0.0.1:1");
    let ran = with_default(subscriber, || {
        Runtime::new()
            .unwrap()
            .block_on(instance.async_moderation_hook(
                ModerationGate {
                    should_run: true,
                    should_proceed: true,
                },
                None,
                &BTreeMap::new(),
            ))
    })
    .expect("a missing messages field does not error");
    assert!(!ran);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Aporia AI: not running guardrail. No messages in data"));
}

#[tokio::test]
async fn aporia_moderation_hook_transforms_roles_before_sending() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body = Arc::new(Mutex::new(Vec::new()));
    let body_for_handler = Arc::clone(&body);
    let answer = move |bytes: Bytes| {
        let body_for_body = Arc::clone(&body_for_handler);
        async move {
            *body_for_body.lock().unwrap() = bytes.to_vec();
            (StatusCode::OK, r#"{"action": "pass"}"#.to_owned())
        }
    };
    let router = Router::new().route("/validate", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let instance = guardrail(&base);
    let raw_messages = vec![message("tool", "call result")];
    let ran = instance
        .async_moderation_hook(
            ModerationGate {
                should_run: true,
                should_proceed: true,
            },
            Some(raw_messages),
            &BTreeMap::new(),
        )
        .await
        .expect("the api call succeeds");
    assert!(ran);
    let captured: Captured = serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.messages[0].role, "other");
}

#[test]
fn aporia_get_config_model_returns_a_type_distinct_from_other_config_models() {
    assert_ne!(
        TypeId::of::<AporiaGuardrailConfigModel>(),
        TypeId::of::<u8>()
    );
}

#[test]
fn aporia_config_model_ui_friendly_name_is_aporia_ai() {
    assert_eq!(AporiaGuardrailConfigModel::ui_friendly_name(), "Aporia AI");
}

#[test]
fn aporia_config_model_fields_default_to_none() {
    let model: AporiaGuardrailConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.api_key, None);
    assert_eq!(model.api_base, None);
}
