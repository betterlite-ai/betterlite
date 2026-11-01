#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::IntoFuture as _;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Content, Message};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::error::{JavelinError, JavelinInitializationError};
use super::wire::{JavelinGuardInput, JavelinGuardRequest, JavelinGuardResponse};
use super::{
    AppliedGuardrailsHeader, CallbackRegistry, JavelinConfigModel, JavelinEventHook,
    JavelinGuardrail, JavelinGuardrailParams, SUPPORTED_EVENT_HOOKS, initialize_guardrail,
};

#[test]
fn javelin_config_model_deserializes_empty_object_with_default_api_version() {
    let model: JavelinConfigModel = serde_json::from_str("{}").expect("empty object");
    assert_eq!(model.api_version, Some("v1".to_owned()));
    assert_eq!(model.guard_name, None);
    assert_eq!(JavelinConfigModel::ui_friendly_name(), "Javelin Guardrails");
}

fn empty_env(_key: &str) -> String {
    String::new()
}

fn env_with_both(key: &str) -> String {
    match key {
        "JAVELIN_API_KEY" => "env-key".to_owned(),
        "JAVELIN_API_BASE" => "https://env.example".to_owned(),
        _ => String::new(),
    }
}

fn user_message(text: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn javelin(params: JavelinGuardrailParams, client: reqwest::Client) -> JavelinGuardrail {
    JavelinGuardrail::new(params, empty_env, client, Duration::from_millis(200))
}

#[derive(Default)]
struct FakeRegistry {
    registered: Vec<Arc<JavelinGuardrail>>,
}

impl CallbackRegistry for FakeRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<JavelinGuardrail>) {
        self.registered.push(guardrail);
    }
}

#[derive(Default)]
struct FakeAppliedGuardrails {
    recorded: Vec<String>,
}

impl AppliedGuardrailsHeader for FakeAppliedGuardrails {
    fn record_applied_guardrail(&mut self, guardrail_name: &str) {
        self.recorded.push(guardrail_name.to_owned());
    }
}

#[test]
fn initialization_without_guard_name_raises_error() {
    let mut registry = FakeRegistry::default();
    let error = initialize_guardrail(
        JavelinGuardrailParams::default(),
        empty_env,
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
        &mut registry,
    )
    .expect_err("missing guard name must fail");
    assert!(matches!(
        error,
        JavelinInitializationError::MissingGuardName
    ));
    assert!(error.to_string().contains("JavelinGuardrailException"));
    assert!(registry.registered.is_empty());
}

#[rstest]
#[case::empty_env_uses_defaults(
    JavelinGuardrailParams::default(),
    empty_env,
    ("", "https://api-dev.javelin.live", "v1", true, "trustsafety", "trustsafety"),
)]
#[case::env_supplies_key_and_base(
    JavelinGuardrailParams::default(),
    env_with_both,
    ("env-key", "https://env.example", "v1", true, "trustsafety", "trustsafety"),
)]
#[case::explicit_values_win_over_env(
    JavelinGuardrailParams {
        api_key: Some(SecretString::from("explicit-key")),
        api_base: Some("https://explicit.example".to_owned()),
        api_version: Some("v2".to_owned()),
        default_on: Some(false),
        guardrail_name: Some("custom".to_owned()),
        javelin_guard_name: Some("custom-guard".to_owned()),
        metadata: None,
        config: None,
        application: None,
    },
    env_with_both,
    ("explicit-key", "https://explicit.example", "v2", false, "custom", "custom-guard"),
)]
#[case::javelin_guard_name_defaults_to_guardrail_name(
    JavelinGuardrailParams {
        api_key: None,
        api_base: None,
        api_version: None,
        default_on: None,
        guardrail_name: Some("custom".to_owned()),
        javelin_guard_name: None,
        metadata: None,
        config: None,
        application: None,
    },
    empty_env,
    ("", "https://api-dev.javelin.live", "v1", true, "custom", "custom"),
)]
fn constructor_resolves_env_defaults_and_overrides(
    #[case] params: JavelinGuardrailParams,
    #[case] get_secret_str: fn(&str) -> String,
    #[case] expected: (&str, &str, &str, bool, &str, &str),
) {
    let guardrail = JavelinGuardrail::new(
        params,
        get_secret_str,
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
    );
    assert_eq!(
        (
            guardrail.api_key.expose_secret(),
            guardrail.api_base.as_str(),
            guardrail.api_version.as_str(),
            guardrail.default_on,
            guardrail.guardrail_name.as_str(),
            guardrail.javelin_guard_name.as_str(),
        ),
        expected
    );
}

#[rstest]
#[case::no_messages(None)]
#[case::empty_messages(Some(Vec::new()))]
#[case::no_user_message(Some(vec![Message {
    role: "assistant".to_owned(),
    content: Some(Content::Text("hi".to_owned())),
    name: None,
    provider_specific_fields: Object::new(),
}]))]
#[tokio::test]
async fn hook_extraction_returns_early_without_messages_or_text(
    #[case] messages: Option<Vec<Message>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let result = guardrail
        .async_pre_call_hook(messages.as_deref(), &mut applied)
        .await;
    result.unwrap();
    assert!(applied.recorded.is_empty());
}

struct CapturedRequest {
    headers: HeaderMap,
    body: String,
}

async fn capturing_server(
    path: &'static str,
    status: StatusCode,
    body: String,
) -> (
    String,
    mpsc::Receiver<CapturedRequest>,
    JoinSet<io::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let (tx, rx) = mpsc::channel(1);
    let router = Router::new().route(
        path,
        post(move |headers: HeaderMap, request_body: String| {
            let request_tx = tx.clone();
            let reply = body.clone();
            async move {
                drop(
                    request_tx
                        .send(CapturedRequest {
                            headers,
                            body: request_body,
                        })
                        .await,
                );
                (status, reply)
            }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, rx, server)
}

#[rstest]
#[case::without_application(None, false)]
#[case::with_application(Some("my-app"), true)]
#[tokio::test]
async fn api_call_sends_headers_to_versioned_guardrail_path(
    #[case] application: Option<&str>,
    #[case] expect_application_header: bool,
) {
    let (base, mut rx, mut server) = capturing_server(
        "/v1/guardrail/trustsafety/apply",
        StatusCode::OK,
        r#"{"assessments":[]}"#.to_owned(),
    )
    .await;
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: Some(SecretString::from("secret-key")),
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: application.map(ToOwned::to_owned),
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let result = guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await;
    result.unwrap();
    let captured = rx.recv().await.expect("the guard API was called");
    assert_eq!(
        captured.headers.get("x-javelin-apikey").unwrap(),
        "secret-key"
    );
    assert_eq!(
        captured.headers.get("x-javelin-application").is_some(),
        expect_application_header
    );
    assert!(captured.body.contains("\"text\":\"hello\""));
    server.shutdown().await;
}

fn metadata_and_config_fixtures() -> (
    BTreeMap<String, GuardrailValue>,
    BTreeMap<String, GuardrailValue>,
) {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "standard_logging_guardrail_information".to_owned(),
        GuardrailValue::Text("drop-me".to_owned()),
    );
    metadata.insert("team".to_owned(), GuardrailValue::Text("a".to_owned()));
    let mut config = BTreeMap::new();
    config.insert("threshold".to_owned(), GuardrailValue::SignedInteger(1));
    (metadata, config)
}

fn expected_request_body(config: BTreeMap<String, GuardrailValue>) -> JavelinGuardRequest {
    let mut metadata = BTreeMap::new();
    metadata.insert("team".to_owned(), GuardrailValue::Text("a".to_owned()));
    JavelinGuardRequest {
        input: JavelinGuardInput {
            text: "hello".to_owned(),
        },
        config,
        metadata,
    }
}

#[tokio::test]
async fn api_call_sends_config_and_metadata_without_the_logging_key() {
    let (base, mut rx, mut server) = capturing_server(
        "/v1/guardrail/trustsafety/apply",
        StatusCode::OK,
        r#"{"assessments":[]}"#.to_owned(),
    )
    .await;
    let (metadata, config) = metadata_and_config_fixtures();
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: Some(metadata),
            config: Some(config.clone()),
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await
        .unwrap();
    let captured = rx.recv().await.expect("the guard API was called");
    let body: JavelinGuardRequest = serde_json::from_str(&captured.body).expect("json body");
    assert_eq!(body, expected_request_body(config));
    server.shutdown().await;
}

#[tokio::test]
async fn api_call_trims_trailing_slash_from_api_base() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}/", listener.local_addr().expect("addr"));
    let (tx, mut rx) = mpsc::channel(1);
    let router = Router::new().fallback(move |uri: Uri| {
        let request_tx = tx.clone();
        async move {
            drop(request_tx.send(uri.path().to_owned()).await);
            (StatusCode::OK, r#"{"assessments":[]}"#)
        }
    });
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let result = guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await;
    result.unwrap();
    let path = rx.recv().await.expect("the guard API was called");
    assert_eq!(path, "/v1/guardrail/trustsafety/apply");
    server.shutdown().await;
}

fn rejecting_response_body(reject_prompt: &str) -> String {
    format!(
        r#"{{"assessments":[{{"trust_safety":{{"request_reject":false,"results":{{}}}}}},{{"prompt_injection":{{"request_reject":true,"results":{{"reject_prompt":"{reject_prompt}"}}}}}},{{"trailing":{{"request_reject":true,"results":{{"reject_prompt":"should be ignored"}}}}}}]}}"#
    )
}

#[rstest]
#[case::explicit_prompt("blocked by policy", "blocked by policy")]
#[case::empty_prompt_falls_back_to_default(
    "",
    "Request blocked by Javelin guardrails due to trustsafety violation."
)]
#[tokio::test]
async fn rejection_stops_at_first_request_reject_assessment(
    #[case] configured_prompt: &str,
    #[case] expected_prompt: &str,
) {
    let (base, _rx, mut server) = capturing_server(
        "/v1/guardrail/trustsafety/apply",
        StatusCode::OK,
        rejecting_response_body(configured_prompt),
    )
    .await;
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let error = guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await
        .expect_err("a rejecting assessment must block the request");
    match error {
        JavelinError::Rejected { reject_prompt, .. } => {
            assert_eq!(reject_prompt, expected_prompt);
        }
        other @ (JavelinError::InvalidApiKeyHeader { .. }
        | JavelinError::InvalidApplicationHeader { .. }
        | JavelinError::Timeout { .. }
        | JavelinError::Request { .. }
        | JavelinError::HttpStatus { .. }
        | JavelinError::ResponseTooLarge { .. }
        | JavelinError::ResponseJson { .. }) => panic!("unexpected error: {other}"),
    }
    assert!(applied.recorded.is_empty());
    server.shutdown().await;
}

#[test]
fn rejection_status_500_detail_includes_reject_prompt_and_response() {
    let response = JavelinGuardResponse {
        assessments: vec![],
    };
    let error = JavelinError::Rejected {
        reject_prompt: "blocked".to_owned(),
        javelin_guardrail_response: response,
    };
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    match &error {
        JavelinError::Rejected {
            reject_prompt,
            javelin_guardrail_response,
        } => {
            assert_eq!(reject_prompt, "blocked");
            assert!(javelin_guardrail_response.assessments.is_empty());
        }
        other @ (JavelinError::InvalidApiKeyHeader { .. }
        | JavelinError::InvalidApplicationHeader { .. }
        | JavelinError::Timeout { .. }
        | JavelinError::Request { .. }
        | JavelinError::HttpStatus { .. }
        | JavelinError::ResponseTooLarge { .. }
        | JavelinError::ResponseJson { .. }) => panic!("unexpected error: {other}"),
    }
    assert!(error.to_string().contains("blocked"));
}

#[tokio::test]
async fn applied_header_records_guardrail_when_request_passes() {
    let (base, _rx, mut server) = capturing_server(
        "/v1/guardrail/trustsafety/apply",
        StatusCode::OK,
        r#"{"assessments":[]}"#.to_owned(),
    )
    .await;
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await
        .expect("a passing assessment must not block the request");
    assert_eq!(applied.recorded, vec!["trustsafety".to_owned()]);
    server.shutdown().await;
}

#[tokio::test]
async fn fail_open_returns_empty_assessments_on_connection_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let result = guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await;
    result.unwrap();
    assert_eq!(applied.recorded, vec!["trustsafety".to_owned()]);
}

async fn slow_server(path: &'static str, delay: Duration) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        path,
        post(move || async move {
            sleep(delay).await;
            (StatusCode::OK, r#"{"assessments":[]}"#)
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

#[tokio::test]
async fn fail_open_returns_empty_assessments_on_timeout() {
    let (base, mut server) = slow_server(
        "/v1/guardrail/trustsafety/apply",
        Duration::from_millis(400),
    )
    .await;
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let result = guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await;
    result.unwrap();
    assert_eq!(applied.recorded, vec!["trustsafety".to_owned()]);
    server.shutdown().await;
}

#[tokio::test]
async fn fail_open_returns_empty_assessments_on_rejecting_server_error() {
    let (base, _rx, mut server) = capturing_server(
        "/v1/guardrail/trustsafety/apply",
        StatusCode::INTERNAL_SERVER_ERROR,
        rejecting_response_body("blocked by policy"),
    )
    .await;
    let guardrail = javelin(
        JavelinGuardrailParams {
            api_key: None,
            api_base: Some(base),
            api_version: None,
            default_on: None,
            guardrail_name: None,
            javelin_guard_name: None,
            metadata: None,
            config: None,
            application: None,
        },
        reqwest::Client::default(),
    );
    let mut applied = FakeAppliedGuardrails::default();
    let result = guardrail
        .async_pre_call_hook(Some(&[user_message("hello")]), &mut applied)
        .await;
    result.unwrap();
    assert_eq!(applied.recorded, vec!["trustsafety".to_owned()]);
    server.shutdown().await;
}

#[test]
fn event_hooks_returns_pre_call_only() {
    assert_eq!(SUPPORTED_EVENT_HOOKS, [JavelinEventHook::PreCall]);
}

#[rstest]
#[case::missing_default_on_resolves_to_false(None, false)]
#[case::explicit_default_on_true_is_kept(Some(true), true)]
#[tokio::test]
async fn registration_adds_guardrail_to_callback_registry(
    #[case] default_on: Option<bool>,
    #[case] expected_default_on: bool,
) {
    let mut registry = FakeRegistry::default();
    let guardrail = initialize_guardrail(
        JavelinGuardrailParams {
            api_key: None,
            api_base: None,
            api_version: None,
            default_on,
            guardrail_name: None,
            javelin_guard_name: Some("trustsafety".to_owned()),
            metadata: None,
            config: None,
            application: None,
        },
        empty_env,
        reqwest::Client::default(),
        super::DEFAULT_TIMEOUT,
        &mut registry,
    )
    .expect("guard name is present");
    assert_eq!(registry.registered.len(), 1);
    assert!(Arc::ptr_eq(&registry.registered[0], &guardrail));
    assert_eq!(guardrail.default_on, expected_default_on);
}

#[test]
fn javelin_guardrail_replays_the_javelin_wire_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/javelin_wire");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        serde_json::from_slice::<JavelinGuardResponse>(&bytes).is_ok()
    }));
}
