#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{
    GUARDRAIL_NAME, GUARDRAIL_PATH, NEXUS_HEADERS, QostodianNexus, QostodianNexusConfigModel,
    QostodianNexusError, QostodianNexusParams, SUPPORTED_EVENT_HOOKS,
};
use crate::generic_guardrail_api::DEFAULT_TIMEOUT;
use crate::generic_guardrail_contracts::{
    GenericGuardrailAPISettings, GuardrailExtraValue, GuardrailInputType, GuardrailMessage,
    GuardrailOptions, GuardrailToolParam, UnreachableFallback,
};
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

type Captured = Arc<Mutex<Option<Vec<u8>>>>;

fn nexus_params() -> QostodianNexusParams {
    QostodianNexusParams {
        api_key: None,
        api_base: None,
        guardrail_name: None,
        extra_headers: None,
        provider_params: BTreeMap::new(),
        settings: GenericGuardrailAPISettings::from_optional(None),
        client: reqwest::Client::default(),
        timeout: None,
    }
}

fn build(params: QostodianNexusParams, environment: &[(&str, &str)]) -> QostodianNexus {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    QostodianNexus::new(params, &lookup).unwrap()
}

fn build_with_base(base: &str) -> QostodianNexus {
    let mut params = nexus_params();
    params.api_base = Some(base.to_owned());
    build(params, &[])
}

fn text_inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

async fn start_server(
    status: u16,
    answer: &str,
) -> (String, Captured, JoinSet<Result<(), IoError>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured: Captured = Arc::new(Mutex::new(None));
    let captured_for_handler = Arc::clone(&captured);
    let body_text = answer.to_owned();
    let handler = move |body: Bytes| {
        let captured_for_body = Arc::clone(&captured_for_handler);
        let reply = body_text.clone();
        async move {
            *captured_for_body.lock().unwrap() = Some(body.to_vec());
            (StatusCode::from_u16(status).unwrap(), reply)
        }
    };
    let router = Router::new().route(GUARDRAIL_PATH, post(handler));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

#[derive(serde::Deserialize)]
struct CapturedMetadata {
    user_api_key_hash: Option<String>,
}

#[derive(serde::Deserialize)]
struct CapturedRequest {
    input_type: GuardrailInputType,
    litellm_call_id: Option<String>,
    litellm_trace_id: Option<String>,
    texts: Option<Vec<String>>,
    request_data: CapturedMetadata,
    request_headers: Option<BTreeMap<String, String>>,
    litellm_version: Option<String>,
}

fn captured_body(captured: &Captured) -> Vec<u8> {
    captured.lock().unwrap().clone().expect("the scan request")
}

#[test]
fn nexus_defaults_api_base_to_the_default_endpoint_with_the_guardrail_path() {
    let nexus = build(nexus_params(), &[]);
    assert_eq!(
        nexus.generic.api_base,
        "http://nexus:8800/beta/litellm_basic_guardrail_api"
    );
}

#[test]
fn nexus_prefers_a_given_api_base_over_the_environment() {
    let mut params = nexus_params();
    params.api_base = Some("https://configured.example".to_owned());
    let nexus = build(
        params,
        &[("QOSTODIAN_NEXUS_API_BASE", "https://env.example")],
    );
    assert_eq!(
        nexus.generic.api_base,
        format!("https://configured.example{GUARDRAIL_PATH}")
    );
}

#[test]
fn nexus_reads_api_base_from_the_environment_when_none_is_given() {
    let nexus = build(
        nexus_params(),
        &[("QOSTODIAN_NEXUS_API_BASE", "https://env.example")],
    );
    assert_eq!(
        nexus.generic.api_base,
        format!("https://env.example{GUARDRAIL_PATH}")
    );
}

#[test]
fn nexus_treats_an_empty_api_base_and_environment_as_unset() {
    let mut params = nexus_params();
    params.api_base = Some(String::new());
    let nexus = build(params, &[("QOSTODIAN_NEXUS_API_BASE", "")]);
    assert_eq!(
        nexus.generic.api_base,
        "http://nexus:8800/beta/litellm_basic_guardrail_api"
    );
}

#[test]
fn nexus_keeps_a_base_that_already_ends_with_the_guardrail_path() {
    let mut params = nexus_params();
    params.api_base = Some(format!("https://configured.example{GUARDRAIL_PATH}"));
    let nexus = build(params, &[]);
    assert_eq!(
        nexus.generic.api_base,
        format!("https://configured.example{GUARDRAIL_PATH}")
    );
}

#[test]
fn nexus_trims_the_trailing_slash_before_appending_the_path() {
    let mut params = nexus_params();
    params.api_base = Some("https://configured.example/".to_owned());
    let nexus = build(params, &[]);
    assert_eq!(
        nexus.generic.api_base,
        format!("https://configured.example{GUARDRAIL_PATH}")
    );
}

#[rstest]
#[case::none(None, "qostodian_nexus")]
#[case::custom(Some("nexus-check"), "nexus-check")]
#[case::explicitly_empty(Some(""), "")]
fn nexus_names_the_guardrail_qostodian_nexus_only_when_none_is_given(
    #[case] guardrail_name: Option<&str>,
    #[case] expected: &str,
) {
    let mut params = nexus_params();
    params.guardrail_name = guardrail_name.map(str::to_owned);
    let nexus = build(params, &[]);
    assert_eq!(nexus.guardrail_name, expected);
}

#[test]
fn nexus_headers_are_the_four_identifiers_first() {
    let nexus = build(nexus_params(), &[]);
    assert_eq!(
        nexus.extra_headers,
        vec![
            "x-qostodian-nexus-identifiers-trace".to_owned(),
            "x-qostodian-nexus-identifiers-source".to_owned(),
            "x-qostodian-nexus-identifiers-container".to_owned(),
            "x-qostodian-nexus-identifiers-identity".to_owned(),
        ]
    );
}

#[rstest]
#[case::keeps_order(vec!["x-caller-one", "x-caller-two"], vec!["x-caller-one", "x-caller-two"])]
#[case::drops_trace(vec!["x-caller-one", "x-qostodian-nexus-identifiers-trace"], vec!["x-caller-one"])]
#[case::drops_source(vec!["x-caller-one", "x-qostodian-nexus-identifiers-source"], vec!["x-caller-one"])]
#[case::drops_container(vec!["x-caller-one", "x-qostodian-nexus-identifiers-container"], vec!["x-caller-one"])]
#[case::drops_identity(vec!["x-caller-one", "x-qostodian-nexus-identifiers-identity"], vec!["x-caller-one"])]
#[case::keeps_caller_duplicates(vec!["x-caller-one", "x-caller-one"], vec!["x-caller-one", "x-caller-one"])]
fn nexus_headers_append_caller_headers_skipping_the_fixed_names(
    #[case] given: Vec<&str>,
    #[case] expected: Vec<&str>,
) {
    let mut params = nexus_params();
    params.extra_headers = Some(given.iter().map(|header| (*header).to_owned()).collect());
    let nexus = build(params, &[]);
    let mut wanted: Vec<String> = NEXUS_HEADERS
        .iter()
        .map(|header| (*header).to_owned())
        .collect();
    wanted.extend(expected.iter().map(|header| (*header).to_owned()));
    assert_eq!(nexus.extra_headers, wanted);
}

#[test]
fn nexus_passes_the_generic_settings_and_timeout_through() {
    let settings = GenericGuardrailAPISettings::from_optional(Some(GuardrailOptions {
        provider_params: None,
        unreachable_fallback: Some(UnreachableFallback::FailOpen),
        fail_on_error: Some(false),
        streaming_end_of_stream_only: None,
        streaming_sampling_rate: None,
        streaming_transform_mode: None,
    }));
    let mut params = nexus_params();
    params.api_key = Some(SecretString::from("nexus-key"));
    params.settings = settings.clone();
    params.provider_params = BTreeMap::from([(
        "threshold".to_owned(),
        GuardrailExtraValue::Decimal(0.5_f64),
    )]);
    params.timeout = Some(2.5_f64);
    let nexus = build(params, &[]);
    assert_eq!(
        nexus.generic.api_key.as_ref().unwrap().expose_secret(),
        "nexus-key"
    );
    assert_eq!(nexus.generic.settings, settings);
    assert_eq!(
        nexus.generic.provider_params,
        BTreeMap::from([(
            "threshold".to_owned(),
            GuardrailExtraValue::Decimal(0.5_f64)
        )])
    );
    assert_eq!(nexus.generic.timeout, Duration::from_millis(2_500));
}

#[test]
fn nexus_defaults_the_timeout_to_the_generic_default() {
    let nexus = build(nexus_params(), &[]);
    assert_eq!(nexus.generic.timeout, DEFAULT_TIMEOUT);
}

#[test]
fn nexus_rejects_an_unrepresentable_timeout() {
    let mut params = nexus_params();
    params.timeout = Some(-1.0_f64);
    let lookup = BTreeMap::<String, String>::new();
    assert!(matches!(
        QostodianNexus::new(params, &lookup),
        Err(QostodianNexusError::UnrepresentableTimeout { .. })
    ));
}

#[test]
fn nexus_supports_pre_call_during_call_and_post_call() {
    let nexus = build(nexus_params(), &[]);
    assert_eq!(
        nexus.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall
        ])
    );
    assert_eq!(SUPPORTED_EVENT_HOOKS.len(), 3);
}

#[test]
fn nexus_config_model_is_the_generic_model_with_the_nexus_api_base() {
    let nexus = build(nexus_params(), &[]);
    let model = nexus.get_config_model().unwrap();
    assert_eq!(
        QostodianNexusConfigModel::ui_friendly_name(),
        "Qostodian Nexus"
    );
    assert_eq!(
        model,
        GuardrailValue::Object(BTreeMap::from([
            ("optional_params".to_owned(), GuardrailValue::Null),
            ("api_base".to_owned(), GuardrailValue::Null),
        ]))
    );
}

#[tokio::test]
async fn apply_guardrail_sends_the_generic_request_and_returns_the_texts() {
    let (base, captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::new();
    let result = nexus
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("allowed");
    let request: CapturedRequest = serde_json::from_slice(&captured_body(&captured)).unwrap();
    assert_eq!(request.input_type, GuardrailInputType::Request);
    assert_eq!(request.texts, Some(vec!["hello".to_owned()]));
    assert_eq!(request.request_headers, None);
    assert_eq!(request.litellm_version.as_deref(), Some("1.105.1"));
    assert_eq!(result.texts, Some(vec!["hello".to_owned()]));
}

#[tokio::test]
async fn apply_guardrail_sends_the_call_and_trace_ids_from_the_log_target() {
    let (base, captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let nexus = build_with_base(&base);
    let mut log_target = GuardrailLogTarget {
        litellm_params: Some(BTreeMap::from([
            (
                "litellm_call_id".to_owned(),
                GuardrailValue::Text("call-1".to_owned()),
            ),
            (
                "litellm_trace_id".to_owned(),
                GuardrailValue::Text("trace-1".to_owned()),
            ),
        ])),
        model_call_params: None,
    };
    let mut request_data = BTreeMap::new();
    nexus
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
            Some(&mut log_target),
        )
        .await
        .expect("allowed");
    let request: CapturedRequest = serde_json::from_slice(&captured_body(&captured)).unwrap();
    assert_eq!(request.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(request.litellm_trace_id.as_deref(), Some("trace-1"));
}

#[tokio::test]
async fn apply_guardrail_uses_the_nexus_headers_as_the_inbound_allowlist() {
    let (base, captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let mut params = nexus_params();
    params.api_base = Some(base);
    params.extra_headers = Some(vec!["x-caller-header".to_owned()]);
    let nexus = build(params, &[]);
    let mut request_data = BTreeMap::from([(
        "headers".to_owned(),
        GuardrailValue::Object(BTreeMap::from([
            (
                "x-caller-header".to_owned(),
                GuardrailValue::Text("forwarded".to_owned()),
            ),
            (
                "x-secret-header".to_owned(),
                GuardrailValue::Text("hidden".to_owned()),
            ),
        ])),
    )]);
    nexus
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("allowed");
    let request: CapturedRequest = serde_json::from_slice(&captured_body(&captured)).unwrap();
    assert_eq!(
        request.request_headers,
        Some(BTreeMap::from([
            ("x-caller-header".to_owned(), "forwarded".to_owned()),
            ("x-secret-header".to_owned(), "[present]".to_owned()),
        ]))
    );
}

#[tokio::test]
async fn apply_guardrail_falls_back_to_the_log_target_headers() {
    let (base, captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let nexus = build_with_base(&base);
    let mut log_target = GuardrailLogTarget {
        litellm_params: Some(BTreeMap::from([
            (
                "metadata".to_owned(),
                GuardrailValue::Object(BTreeMap::from([(
                    "headers".to_owned(),
                    GuardrailValue::Object(BTreeMap::from([(
                        "x-log-header".to_owned(),
                        GuardrailValue::Text("logged".to_owned()),
                    )])),
                )])),
            ),
            (
                "extra_headers".to_owned(),
                GuardrailValue::Array(vec![GuardrailValue::Text("x-log-header".to_owned())]),
            ),
        ])),
        model_call_params: None,
    };
    let mut request_data = BTreeMap::new();
    nexus
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
            Some(&mut log_target),
        )
        .await
        .expect("allowed");
    let request: CapturedRequest = serde_json::from_slice(&captured_body(&captured)).unwrap();
    assert_eq!(
        request.request_headers,
        Some(BTreeMap::from([(
            "x-log-header".to_owned(),
            "logged".to_owned()
        )]))
    );
}

#[tokio::test]
async fn apply_guardrail_extracts_the_user_api_key_metadata() {
    let (base, captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::from([
        (
            "metadata".to_owned(),
            GuardrailValue::Object(BTreeMap::from([
                (
                    "user_api_key_hash".to_owned(),
                    GuardrailValue::Text("old-hash".to_owned()),
                ),
                (
                    "user_api_key_token".to_owned(),
                    GuardrailValue::Text("token".to_owned()),
                ),
            ])),
        ),
        (
            "litellm_metadata".to_owned(),
            GuardrailValue::Object(BTreeMap::from([(
                "user_api_key_hash".to_owned(),
                GuardrailValue::Text("preferred-hash".to_owned()),
            )])),
        ),
    ]);
    nexus
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("allowed");
    let request: CapturedRequest = serde_json::from_slice(&captured_body(&captured)).unwrap();
    assert_eq!(
        request.request_data.user_api_key_hash.as_deref(),
        Some("token")
    );
}

#[tokio::test]
async fn apply_guardrail_returns_rewritten_texts_from_the_scan() {
    let (base, _captured, _server) = start_server(
        200,
        r#"{"action": "GUARDRAIL_INTERVENED", "texts": ["clean text"]}"#,
    )
    .await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::new();
    let result = nexus
        .apply_guardrail(
            text_inputs(&["raw text"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("allowed");
    assert_eq!(result.texts, Some(vec!["clean text".to_owned()]));
}

#[tokio::test]
async fn apply_guardrail_keeps_the_original_images_and_tools_when_the_scan_adds_none() {
    let (base, _captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let nexus = build_with_base(&base);
    let mut inputs = text_inputs(&["hello"]);
    inputs.images = Some(vec!["data:image/png;base64,AA".to_owned()]);
    inputs.tools = Some(vec![GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    }]);
    let mut request_data = BTreeMap::new();
    let result = nexus
        .apply_guardrail(
            inputs.clone(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("allowed");
    assert_eq!(result.images, inputs.images);
    assert_eq!(result.tools, inputs.tools);
    assert_eq!(result.texts, inputs.texts);
}

#[tokio::test]
async fn apply_guardrail_raises_a_guardrail_failure_when_blocked() {
    let (base, _captured, _server) = start_server(
        200,
        r#"{"action": "BLOCKED", "blocked_reason": "pii found"}"#,
    )
    .await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::new();
    let failure = nexus
        .apply_guardrail(
            text_inputs(&["secret"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("blocked");
    assert_eq!(
        failure,
        GuardrailFailure::GuardrailRaised {
            guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
            message: "pii found".to_owned(),
            wrap_default_message: false,
            status_code: 400,
            blocked_content: true,
        }
    );
}

#[tokio::test]
async fn apply_guardrail_uses_the_default_message_for_an_unexplained_block() {
    let (base, _captured, _server) = start_server(200, r#"{"action": "BLOCKED"}"#).await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::new();
    let failure = nexus
        .apply_guardrail(
            text_inputs(&["secret"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("blocked");
    assert_eq!(
        failure,
        GuardrailFailure::GuardrailRaised {
            guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
            message: "Content violates policy".to_owned(),
            wrap_default_message: false,
            status_code: 400,
            blocked_content: true,
        }
    );
}

#[tokio::test]
async fn apply_guardrail_reports_a_scan_error_as_an_other_failure() {
    let (base, _captured, _server) = start_server(500, r#"{"error": "boom"}"#).await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::new();
    let failure = nexus
        .apply_guardrail(
            text_inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("fails closed");
    assert!(matches!(failure, GuardrailFailure::Other(_)));
}

#[tokio::test]
async fn apply_guardrail_fails_open_with_the_request_fields_but_drops_tool_calls_and_model() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut params = nexus_params();
    params.api_base = Some(base);
    params.settings = GenericGuardrailAPISettings::from_optional(Some(GuardrailOptions {
        provider_params: None,
        unreachable_fallback: Some(UnreachableFallback::FailOpen),
        fail_on_error: Some(true),
        streaming_end_of_stream_only: None,
        streaming_sampling_rate: None,
        streaming_transform_mode: None,
    }));
    let nexus = build(params, &[]);
    let mut inputs = text_inputs(&["hello"]);
    inputs.images = Some(vec!["data:image/png;base64,AA".to_owned()]);
    inputs.tools = Some(vec![GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra: BTreeMap::new(),
    }]);
    inputs.tool_calls = Some(vec![GuardrailExtraValue::Object(BTreeMap::new())]);
    inputs.structured_messages = Some(vec![GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::new(),
    }]);
    inputs.model = Some("gpt-4o".to_owned());
    inputs.stream_holdback_chars = Some(vec![2]);
    let mut request_data = BTreeMap::new();
    let result = nexus
        .apply_guardrail(
            inputs.clone(),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect("fails open");
    assert_eq!(result.texts, inputs.texts);
    assert_eq!(result.images, inputs.images);
    assert_eq!(result.tools, inputs.tools);
    assert_eq!(result.structured_messages, inputs.structured_messages);
    assert_eq!(result.tool_calls, None);
    assert_eq!(result.model, None);
    assert_eq!(result.stream_holdback_chars, None);
}

#[tokio::test]
async fn apply_guardrail_drops_empty_images_and_tools_like_litellm() {
    let (base, _captured, _server) = start_server(200, r#"{"action": "NONE"}"#).await;
    let nexus = build_with_base(&base);
    let mut inputs = text_inputs(&["hello"]);
    inputs.images = Some(Vec::new());
    inputs.tools = Some(Vec::new());
    let mut request_data = BTreeMap::new();
    let result = nexus
        .apply_guardrail(inputs, &mut request_data, GuardrailInputType::Request, None)
        .await
        .expect("allowed");
    assert_eq!(result.images, None);
    assert_eq!(result.tools, None);
    assert_eq!(result.texts, Some(vec!["hello".to_owned()]));
}

#[tokio::test]
async fn apply_guardrail_uses_the_default_message_for_an_empty_block_reason() {
    let (base, _captured, _server) =
        start_server(200, r#"{"action": "BLOCKED", "blocked_reason": ""}"#).await;
    let nexus = build_with_base(&base);
    let mut request_data = BTreeMap::new();
    let failure = nexus
        .apply_guardrail(
            text_inputs(&["secret"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .expect_err("blocked");
    assert_eq!(
        failure,
        GuardrailFailure::GuardrailRaised {
            guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
            message: "Content violates policy".to_owned(),
            wrap_default_message: false,
            status_code: 400,
            blocked_content: true,
        }
    );
}
