#![cfg(test)]

use std::any::TypeId;
use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::Result as IoResult;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{
    GuardrailsAiConfigModel, GuardrailsAiGuardrail, GuardrailsAiGuardrailError, InputFormat,
    SUPPORTED_EVENT_HOOKS, Settings,
};

fn settings(api_base: &str, input_format: InputFormat) -> Settings {
    Settings {
        guard_name: "my-guard".to_owned(),
        api_base: api_base.to_owned(),
        input_format,
    }
}

fn guardrail(api_base: &str, input_format: InputFormat) -> GuardrailsAiGuardrail {
    GuardrailsAiGuardrail {
        settings: settings(api_base, input_format),
        client: reqwest::Client::default(),
        timeout: Duration::from_secs(30),
    }
}

#[test]
fn guardrails_ai_supported_event_hooks_are_post_call_pre_call_and_logging_only() {
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).unwrap(),
        r#"["post_call","pre_call","logging_only"]"#
    );
}

#[test]
fn guardrails_ai_settings_resolve_errors_when_guard_name_is_none() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let error = Settings::resolve(None, None, InputFormat::LlmOutput, &lookup)
        .expect_err("a missing guard name raises");
    assert!(matches!(
        error,
        GuardrailsAiGuardrailError::MissingGuardName
    ));
}

#[test]
fn guardrails_ai_settings_resolve_defaults_the_api_base_when_absent_everywhere() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let resolved = Settings::resolve(
        Some("guard".to_owned()),
        None,
        InputFormat::LlmOutput,
        &lookup,
    )
    .expect("guard name is present");
    assert_eq!(resolved.api_base, "http://0.0.0.0:8000");
}

#[test]
fn guardrails_ai_settings_resolve_falls_back_to_the_guardrails_ai_api_base_environment_variable() {
    let lookup = BTreeMap::from([(
        "GUARDRAILS_AI_API_BASE".to_owned(),
        "http://env-base:9000".to_owned(),
    )]);
    let resolved = Settings::resolve(
        Some("guard".to_owned()),
        None,
        InputFormat::LlmOutput,
        &lookup,
    )
    .expect("guard name is present");
    assert_eq!(resolved.api_base, "http://env-base:9000");
}

#[test]
fn guardrails_ai_settings_resolve_prefers_the_constructor_api_base_over_environment() {
    let lookup = BTreeMap::from([(
        "GUARDRAILS_AI_API_BASE".to_owned(),
        "http://env-base:9000".to_owned(),
    )]);
    let resolved = Settings::resolve(
        Some("guard".to_owned()),
        Some("http://explicit:1234".to_owned()),
        InputFormat::LlmOutput,
        &lookup,
    )
    .expect("guard name is present");
    assert_eq!(resolved.api_base, "http://explicit:1234");
}

#[derive(Deserialize)]
struct CapturedLlmOutputRequest {
    #[serde(rename = "llmOutput")]
    llm_output: String,
}

#[derive(Deserialize)]
struct CapturedDynamicParams {
    #[serde(default)]
    project_id: Option<String>,
}

async fn mock_validate_server(
    status: StatusCode,
    response_body: String,
) -> (String, Arc<Mutex<Vec<u8>>>, JoinSet<IoResult<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body = Arc::new(Mutex::new(Vec::new()));
    let body_for_handler = Arc::clone(&body);
    let answer = move |bytes: Bytes| {
        let body_for_body = Arc::clone(&body_for_handler);
        async move {
            *body_for_body.lock().unwrap() = bytes.to_vec();
            (status, response_body)
        }
    };
    let router = Router::new().route("/guards/my-guard/validate", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, body, server)
}

#[tokio::test]
async fn guardrails_ai_make_api_request_posts_the_llm_output_field() {
    let (base, body, _server) =
        mock_validate_server(StatusCode::OK, r#"{"validationPassed": true}"#.to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    instance
        .make_guardrails_ai_api_request("hello world", &BTreeMap::new())
        .await
        .expect("validation passes");
    let captured: CapturedLlmOutputRequest =
        serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.llm_output, "hello world");
}

#[tokio::test]
async fn guardrails_ai_make_api_request_merges_dynamic_params_into_the_json_body() {
    let (base, body, _server) =
        mock_validate_server(StatusCode::OK, r#"{"validationPassed": true}"#.to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let dynamic_params = BTreeMap::from([(
        "project_id".to_owned(),
        GuardrailValue::Text("p-1".to_owned()),
    )]);
    instance
        .make_guardrails_ai_api_request("hello", &dynamic_params)
        .await
        .expect("validation passes");
    let captured: CapturedDynamicParams =
        serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.project_id, Some("p-1".to_owned()));
}

#[tokio::test]
async fn guardrails_ai_make_api_request_raises_when_validation_passed_is_false() {
    let (base, _body, _server) =
        mock_validate_server(StatusCode::OK, r#"{"validationPassed": false}"#.to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let error = instance
        .make_guardrails_ai_api_request("hello", &BTreeMap::new())
        .await
        .expect_err("a failed validation raises");
    assert!(matches!(error, GuardrailsAiGuardrailError::Blocked { .. }));
}

#[tokio::test]
async fn guardrails_ai_make_api_request_does_not_raise_when_validation_passed_is_absent() {
    let (base, _body, _server) = mock_validate_server(StatusCode::OK, "{}".to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    instance
        .make_guardrails_ai_api_request("hello", &BTreeMap::new())
        .await
        .expect("an absent validationPassed field does not block");
}

#[tokio::test]
async fn guardrails_ai_make_api_request_rejects_a_non_success_status() {
    let (base, _body, _server) =
        mock_validate_server(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let result = instance
        .make_guardrails_ai_api_request("hello", &BTreeMap::new())
        .await;
    assert!(matches!(
        result,
        Err(GuardrailsAiGuardrailError::HttpStatus { status })
            if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[derive(Deserialize)]
struct CapturedInputData {
    name: String,
    shape: Vec<u32>,
    data: Vec<String>,
    datatype: String,
}

#[derive(Deserialize)]
struct CapturedInputsRequest {
    inputs: Vec<CapturedInputData>,
}

#[tokio::test]
async fn guardrails_ai_pre_call_request_sends_an_inputs_list_with_one_bytes_element() {
    let (base, body, _server) = mock_validate_server(
        StatusCode::OK,
        r#"{"modelname": "m", "modelversion": "1", "outputs": [{"name": "text", "shape": [1], "data": ["validated"], "datatype": "BYTES"}]}"#
            .to_owned(),
    )
    .await;
    let instance = guardrail(&base, InputFormat::Inputs);
    let result = instance
        .make_pre_call_request("original text")
        .await
        .expect("a successful pre-call response returns the first output");
    assert_eq!(result, "validated");
    let captured: CapturedInputsRequest =
        serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.inputs.len(), 1);
    assert_eq!(captured.inputs[0].name, "text");
    assert_eq!(captured.inputs[0].shape, vec![1]);
    assert_eq!(captured.inputs[0].data, vec!["original text".to_owned()]);
    assert_eq!(captured.inputs[0].datatype, "BYTES");
}

#[tokio::test]
async fn guardrails_ai_pre_call_request_treats_status_400_as_a_blocked_violation() {
    let (base, _body, _server) = mock_validate_server(
        StatusCode::BAD_REQUEST,
        r#"{"error": "blocked"}"#.to_owned(),
    )
    .await;
    let instance = guardrail(&base, InputFormat::Inputs);
    let error = instance
        .make_pre_call_request("text")
        .await
        .expect_err("status 400 is a blocked violation");
    assert!(matches!(
        error,
        GuardrailsAiGuardrailError::PreCallBlocked { .. }
    ));
}

#[tokio::test]
async fn guardrails_ai_pre_call_request_rejects_other_non_success_statuses() {
    let (base, _body, _server) =
        mock_validate_server(StatusCode::INTERNAL_SERVER_ERROR, "oops".to_owned()).await;
    let instance = guardrail(&base, InputFormat::Inputs);
    let result = instance.make_pre_call_request("text").await;
    assert!(matches!(
        result,
        Err(GuardrailsAiGuardrailError::HttpStatus { status })
            if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[tokio::test]
async fn guardrails_ai_process_input_returns_none_when_last_user_message_is_none() {
    let instance = guardrail("http://127.0.0.1:1", InputFormat::LlmOutput);
    let result = instance
        .process_input(None, &BTreeMap::new())
        .await
        .expect("a missing message does not error");
    assert_eq!(result, None);
}

#[tokio::test]
async fn guardrails_ai_process_input_uses_validated_output_when_present() {
    let (base, _body, _server) = mock_validate_server(
        StatusCode::OK,
        r#"{"validationPassed": true, "validatedOutput": "fixed"}"#.to_owned(),
    )
    .await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let result = instance
        .process_input(Some("original".to_owned()), &BTreeMap::new())
        .await
        .expect("validation succeeds");
    assert_eq!(result, Some("fixed".to_owned()));
}

#[tokio::test]
async fn guardrails_ai_process_input_falls_back_to_raw_llm_output_when_validated_output_is_absent()
{
    let (base, _body, _server) = mock_validate_server(
        StatusCode::OK,
        r#"{"validationPassed": true, "rawLlmOutput": "raw"}"#.to_owned(),
    )
    .await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let result = instance
        .process_input(Some("original".to_owned()), &BTreeMap::new())
        .await
        .expect("validation succeeds");
    assert_eq!(result, Some("raw".to_owned()));
}

#[tokio::test]
async fn guardrails_ai_process_input_falls_back_to_the_original_text_when_both_outputs_are_absent()
{
    let (base, _body, _server) =
        mock_validate_server(StatusCode::OK, r#"{"validationPassed": true}"#.to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let result = instance
        .process_input(Some("original".to_owned()), &BTreeMap::new())
        .await
        .expect("validation succeeds");
    assert_eq!(result, Some("original".to_owned()));
}

#[tokio::test]
async fn guardrails_ai_process_input_falls_back_to_the_original_text_when_validated_output_is_empty()
 {
    let (base, _body, _server) = mock_validate_server(
        StatusCode::OK,
        r#"{"validationPassed": true, "validatedOutput": "", "rawLlmOutput": ""}"#.to_owned(),
    )
    .await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let result = instance
        .process_input(Some("original".to_owned()), &BTreeMap::new())
        .await
        .expect("validation succeeds");
    assert_eq!(result, Some("original".to_owned()));
}

#[tokio::test]
async fn guardrails_ai_process_input_uses_the_inputs_format_when_configured() {
    let (base, _body, _server) = mock_validate_server(
        StatusCode::OK,
        r#"{"modelname": "m", "modelversion": "1", "outputs": [{"name": "text", "shape": [1], "data": ["validated"], "datatype": "BYTES"}]}"#
            .to_owned(),
    )
    .await;
    let instance = guardrail(&base, InputFormat::Inputs);
    let result = instance
        .process_input(Some("original".to_owned()), &BTreeMap::new())
        .await
        .expect("validation succeeds");
    assert_eq!(result, Some("validated".to_owned()));
}

#[tokio::test]
async fn guardrails_ai_post_call_hook_skips_without_calling_the_api_when_should_run_is_false() {
    let instance = guardrail("http://127.0.0.1:1", InputFormat::LlmOutput);
    let ran = instance
        .async_post_call_success_hook(false, Some("reply".to_owned()), &BTreeMap::new())
        .await
        .expect("skipped guardrails do not error");
    assert!(!ran);
}

#[tokio::test]
async fn guardrails_ai_post_call_hook_skips_without_calling_the_api_when_content_is_absent() {
    let instance = guardrail("http://127.0.0.1:1", InputFormat::LlmOutput);
    let ran = instance
        .async_post_call_success_hook(true, None, &BTreeMap::new())
        .await
        .expect("a missing response content skips the call");
    assert!(!ran);
}

#[tokio::test]
async fn guardrails_ai_post_call_hook_skips_without_calling_the_api_when_content_is_empty() {
    let instance = guardrail("http://127.0.0.1:1", InputFormat::LlmOutput);
    let ran = instance
        .async_post_call_success_hook(true, Some(String::new()), &BTreeMap::new())
        .await
        .expect("an empty response content skips the call");
    assert!(!ran);
}

#[tokio::test]
async fn guardrails_ai_post_call_hook_calls_the_api_with_the_response_content() {
    let (base, body, _server) =
        mock_validate_server(StatusCode::OK, r#"{"validationPassed": true}"#.to_owned()).await;
    let instance = guardrail(&base, InputFormat::LlmOutput);
    let ran = instance
        .async_post_call_success_hook(true, Some("final reply".to_owned()), &BTreeMap::new())
        .await
        .expect("the api call succeeds");
    assert!(ran);
    let captured: CapturedLlmOutputRequest =
        serde_json::from_slice(&body.lock().unwrap().clone()).unwrap();
    assert_eq!(captured.llm_output, "final reply");
}

#[test]
fn guardrails_ai_get_config_model_returns_a_type_distinct_from_other_config_models() {
    assert_ne!(TypeId::of::<GuardrailsAiConfigModel>(), TypeId::of::<u8>());
}

#[test]
fn guardrails_ai_config_model_ui_friendly_name_is_guardrails_ai() {
    assert_eq!(GuardrailsAiConfigModel::ui_friendly_name(), "Guardrails AI");
}

#[test]
fn guardrails_ai_config_model_defaults_guard_name_and_api_base_to_none_and_format_to_llm_output() {
    let model: GuardrailsAiConfigModel = serde_json::from_str("{}").unwrap();
    assert_eq!(model.guard_name, None);
    assert_eq!(model.api_base, None);
    assert_eq!(model.guardrails_ai_api_input_format, InputFormat::LlmOutput);
}
