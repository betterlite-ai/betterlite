#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};
use thiserror::Error;

use super::async_invoke::{self, AsyncInvokeOutput, AsyncInvokeS3Output, AsyncInvokeStatus};
use super::connection::{self, CancelParams, CancelTimeout, StructuredTimeout, TimeoutValue};
use super::service::BatchProviderCancel;
use super::{
    AzureCancelCall, BatchCancelDispatch, BatchProviderCancelError, BatchProviderCancelInput,
    BatchProviderCancelResult, BedrockCancelCall, CancelBatchCall, CancelCall, CancelProvider,
    OpenAiCancelCall, VertexCancelCall, XaiCancelCall,
};
use crate::api::api_boundaries::ApiLimits;

const SECOND: i64 = 1_704_067_200;

fn base_settings() -> Settings {
    Settings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn base_params() -> CancelParams {
    CancelParams {
        api_base: None,
        api_key: None,
        api_version: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        azure_ad_token: None,
        aws_region_name: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
    }
}

fn answer(status: BatchStatus) -> BatchObject {
    BatchObject {
        id: "batch_1".to_owned(),
        completion_window: "24h".to_owned(),
        created_at: SECOND,
        endpoint: "/v1/chat/completions".to_owned(),
        input_file_id: "file_1".to_owned(),
        object: BatchObjectKind::Batch,
        status,
        cancelled_at: None,
        cancelling_at: Some(SECOND),
        completed_at: None,
        error_file_id: None,
        errors: None,
        expired_at: None,
        expires_at: None,
        failed_at: None,
        finalizing_at: None,
        in_progress_at: None,
        metadata: None,
        model: None,
        output_file_id: None,
        request_counts: None,
        usage: None,
        provider_fields: Map::new(),
    }
}

#[derive(Debug, Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

struct Scripted {
    calls: Arc<Mutex<Vec<Value>>>,
    answer: BatchObject,
}

impl Scripted {
    fn record(&self, call: Value) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn scripted(status: BatchStatus) -> (Scripted, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dispatch = Scripted {
        calls: Arc::clone(&calls),
        answer: answer(status),
    };
    (dispatch, calls)
}

// the five provider branches differ only in their recorded fields, so one macro keeps them one shape
macro_rules! scripted_branches {
    ($(
        $method:ident($call_type:ident): $branch:literal => {
            $($key:literal: $value:expr),* $(,)?
        };
    )*) => {
        impl BatchCancelDispatch for Scripted {
            type Error = ScriptFailure;

            $(
                fn $method(
                    &self,
                    call: $call_type<'_>,
                ) -> impl Future<Output = Result<BatchObject, ScriptFailure>> + Send {
                    self.record(json!({
                        "branch": $branch,
                        $($key: $value,)*
                    }));
                    let reply = self.reply();
                    async move { reply }
                }
            )*
        }
    };
}

scripted_branches! {
    openai(OpenAiCancelCall): "openai" => {
        "is_async": call.is_async,
        "request": recorded_request(call.request),
        "api_base": call.api_base,
        "api_key": call.api_key.map(|key| key.expose_secret()),
        "organization": call.organization,
        "timeout": call.timeout,
        "max_retries": call.max_retries,
    };
    azure(AzureCancelCall): "azure" => {
        "is_async": call.is_async,
        "request": recorded_request(call.request),
        "api_base": call.api_base,
        "api_key": call.api_key.map(|key| key.expose_secret()),
        "api_version": call.api_version,
        "timeout": call.timeout,
        "max_retries": call.max_retries,
    };
    vertex(VertexCancelCall): "vertex" => {
        "is_async": call.is_async,
        "batch_id": call.batch_id,
        "api_base": call.api_base,
        "vertex_project": call.vertex_project,
        "vertex_location": call.vertex_location,
        "vertex_credentials": call.vertex_credentials.map(|token| token.expose_secret()),
        "timeout": call.timeout,
        "max_retries": call.max_retries,
    };
    xai(XaiCancelCall): "xai" => {
        "is_async": call.is_async,
        "batch_id": call.batch_id,
        "api_base": call.api_base,
        "api_key": call.api_key.map(|key| key.expose_secret()),
        "timeout": call.timeout,
    };
    bedrock(BedrockCancelCall): "bedrock" => {
        "batch_id": call.batch_id,
        "params": call.params,
    };
}

fn recorded_request(request: &BatchIdRequest) -> Value {
    json!({
        "batch_id": request.batch_id,
        "extra_headers": request.extra_headers,
        "extra_body": request.extra_body,
    })
}

impl Scripted {
    fn reply(&self) -> Result<BatchObject, ScriptFailure> {
        Ok(self.answer.clone())
    }
}

fn service(settings: Settings, dispatch: Scripted) -> BatchProviderCancel<Scripted> {
    BatchProviderCancel {
        clock: Arc::new(|| SECOND),
        dispatch,
        limits: ApiLimits::default(),
        settings,
    }
}

fn drive(
    service: &BatchProviderCancel<Scripted>,
    input: &BatchProviderCancelInput,
) -> Result<BatchProviderCancelResult, BatchProviderCancelError<ScriptFailure>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(service.run(input))
}

fn recorded(calls: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn request(provider: &str, params: CancelParams, call: CancelCall) -> BatchProviderCancelInput {
    BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider::from_name(provider),
        extra_headers: None,
        extra_body: None,
        params,
        call,
    }))
}

#[rstest]
#[case::openai("openai", json!({"branch": "openai", "is_async": false, "request": {"batch_id": "batch_1", "extra_headers": null, "extra_body": null}, "api_base": "https://api.openai.com/v1", "api_key": "sk-configured", "organization": null, "timeout": 600.0, "max_retries": null}))]
#[case::hosted_vllm("hosted_vllm", json!({"branch": "openai", "is_async": false, "request": {"batch_id": "batch_1", "extra_headers": null, "extra_body": null}, "api_base": "https://api.openai.com/v1", "api_key": "sk-configured", "organization": null, "timeout": 600.0, "max_retries": null}))]
#[case::litellm_proxy("litellm_proxy", json!({"branch": "openai", "is_async": false, "request": {"batch_id": "batch_1", "extra_headers": null, "extra_body": null}, "api_base": "https://api.openai.com/v1", "api_key": "sk-configured", "organization": null, "timeout": 600.0, "max_retries": null}))]
#[case::azure("azure", json!({"branch": "azure", "is_async": false, "request": {"batch_id": "batch_1", "extra_headers": null, "extra_body": null}, "api_base": null, "api_key": "sk-configured", "api_version": null, "azure_ad_token": null, "timeout": 600.0, "max_retries": null}))]
#[case::vertex_ai("vertex_ai", json!({"branch": "vertex", "is_async": false, "batch_id": "batch_1", "api_base": null, "vertex_project": null, "vertex_location": null, "vertex_credentials": null, "timeout": 600.0, "max_retries": null}))]
#[case::xai("xai", json!({"branch": "xai", "is_async": false, "batch_id": "batch_1", "api_base": null, "api_key": null, "timeout": 600.0}))]
fn batch_cancel(#[case] provider: &str, #[case] expected: Value) {
    let mut settings = base_settings();
    settings.api_key = Some(SecretString::from("sk-configured".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    let result = drive(
        &service,
        &request(provider, base_params(), CancelCall::Sync),
    )
    .expect("the provider answered");
    assert_eq!(recorded(&calls), vec![expected]);
    assert_eq!(result.batch, answer(BatchStatus::Cancelling));
    assert_eq!(result.model, None);
}

#[test]
fn batch_cancel_forwards_the_raw_parameters_to_bedrock() {
    let mut params = base_params();
    params.aws_region_name = Some("us-east-1".to_owned());
    params.aws_access_key_id = Some("access".to_owned());
    params.aws_session_token = Some(SecretString::from("session".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    drive(&service, &request("bedrock", params, CancelCall::Sync)).expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["branch"], json!("bedrock"));
    assert_eq!(entries[0]["params"]["aws_region_name"], json!("us-east-1"));
    assert_eq!(entries[0]["params"]["aws_session_token"], json!("session"));
    assert_eq!(entries[0]["params"]["timeout"], Value::Null);
}

#[test]
fn batch_cancel_prefers_call_parameters_over_settings() {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.organization = Some("org-settings".to_owned());
    settings.api_key = Some(SecretString::from("sk-settings".to_owned()));
    let mut params = base_params();
    params.api_base = Some("https://param.example".to_owned());
    params.organization = Some("org-param".to_owned());
    params.api_key = Some(SecretString::from("sk-param".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(&service, &request("openai", params, CancelCall::Sync)).expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["api_base"], json!("https://param.example"));
    assert_eq!(entries[0]["organization"], json!("org-param"));
    assert_eq!(entries[0]["api_key"], json!("sk-param"));
}

#[test]
fn batch_cancel_reads_configured_settings() {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.openai_key = Some(SecretString::from("sk-openai".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(
        &service,
        &request("openai", base_params(), CancelCall::Sync),
    )
    .expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["api_base"], json!("https://settings.example"));
    assert_eq!(entries[0]["api_key"], json!("sk-openai"));
}

#[rstest]
#[case::openai_base_url(
    None,
    Some("https://base-url.example"),
    None,
    "https://base-url.example"
)]
#[case::openai_api_base(
    None,
    None,
    Some("https://api-base.example"),
    "https://api-base.example"
)]
#[case::base_url_wins(
    Some("https://first.example"),
    Some("https://base-url.example"),
    None,
    "https://first.example"
)]
fn batch_cancel_environment_fallbacks(
    #[case] base: Option<&str>,
    #[case] base_url: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let mut settings = base_settings();
    settings.api_base = base.map(str::to_owned);
    settings.openai_base_url = base_url.map(str::to_owned);
    settings.openai_api_base = api_base.map(str::to_owned);
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(
        &service,
        &request("openai", base_params(), CancelCall::Sync),
    )
    .expect("answered");
    assert_eq!(recorded(&calls)[0]["api_base"], json!(expected));
}

#[test]
fn batch_cancel_skips_empty_bases_and_credentials() {
    let mut settings = base_settings();
    settings.api_base = Some(String::new());
    settings.api_key = Some(SecretString::from(String::new()));
    settings.openai_key = Some(SecretString::from(String::new()));
    settings.openai_api_key = Some(SecretString::from("sk-openai-api".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(
        &service,
        &request("openai", base_params(), CancelCall::Sync),
    )
    .expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["api_base"], json!("https://api.openai.com/v1"));
    assert_eq!(entries[0]["api_key"], json!("sk-openai-api"));
}

#[test]
fn batch_cancel_resolves_azure_settings() {
    let mut settings = base_settings();
    settings.azure_api_base = Some("https://azure.example".to_owned());
    settings.azure_api_version = Some("2024-06-01".to_owned());
    settings.azure_key = Some(SecretString::from("azure-key".to_owned()));
    settings.azure_openai_api_key = Some(SecretString::from("azure-openai-key".to_owned()));
    settings.azure_api_key = Some(SecretString::from("azure-api-key".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(&service, &request("azure", base_params(), CancelCall::Sync)).expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["api_base"], json!("https://azure.example"));
    assert_eq!(entries[0]["api_version"], json!("2024-06-01"));
    assert_eq!(entries[0]["api_key"], json!("azure-key"));
}

#[test]
fn batch_cancel_resolves_vertex_settings() {
    let mut settings = base_settings();
    settings.vertex_project = Some("project-settings".to_owned());
    settings.vertexai_project = Some("project-env".to_owned());
    settings.vertexai_location = Some("location-env".to_owned());
    settings.vertexai_credentials = Some(SecretString::from("vertex-token".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(
        &service,
        &request("vertex_ai", base_params(), CancelCall::Sync),
    )
    .expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["vertex_project"], json!("project-settings"));
    assert_eq!(entries[0]["vertex_location"], json!("location-env"));
    assert_eq!(entries[0]["vertex_credentials"], json!("vertex-token"));
}

#[test]
fn batch_cancel_xai_reads_only_the_call_parameters() {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.openai_key = Some(SecretString::from("sk-openai".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    drive(&service, &request("xai", base_params(), CancelCall::Sync)).expect("answered");
    let entries = recorded(&calls);
    assert_eq!(entries[0]["api_base"], Value::Null);
    assert_eq!(entries[0]["api_key"], Value::Null);
}

#[rstest]
#[case::default_timeout(None, None, json!(600.0))]
#[case::request_timeout(None, Some(42.5), json!(42.5))]
#[case::configured_timeout(Some(TimeoutValue::Seconds(12.0)), Some(42.5), json!(12.0))]
#[case::text_timeout(Some(TimeoutValue::Text(" 7.5 ".to_owned())), None, json!(7.5))]
#[case::zero_timeout(Some(TimeoutValue::Seconds(0.0)), Some(42.5), json!(0.0))]
fn batch_cancel_normalizes_the_timeout(
    #[case] timeout: Option<TimeoutValue>,
    #[case] request_timeout: Option<f64>,
    #[case] expected: Value,
) {
    let mut params = base_params();
    params.timeout = timeout;
    params.request_timeout = request_timeout;
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    drive(&service, &request("openai", params, CancelCall::Sync)).expect("answered");
    assert_eq!(recorded(&calls)[0]["timeout"], expected);
}

#[test]
fn batch_cancel_keeps_a_structured_timeout_for_supported_providers() {
    let mut params = base_params();
    params.timeout = Some(TimeoutValue::Structured(StructuredTimeout {
        connect: Some(1.0),
        read: Some(30.0),
        write: Some(2.0),
        pool: Some(3.0),
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    drive(&service, &request("openai", params, CancelCall::Sync)).expect("answered");
    assert_eq!(
        recorded(&calls)[0]["timeout"],
        json!({"connect": 1.0, "read": 30.0, "write": 2.0, "pool": 3.0})
    );
}

#[test]
fn batch_cancel_collapses_a_structured_timeout_for_other_providers() {
    let mut params = base_params();
    params.timeout = Some(TimeoutValue::Structured(StructuredTimeout {
        connect: Some(1.0),
        read: Some(30.0),
        write: Some(2.0),
        pool: Some(3.0),
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    drive(&service, &request("vertex_ai", params, CancelCall::Sync)).expect("answered");
    assert_eq!(recorded(&calls)[0]["timeout"], json!(30.0));
}

#[test]
fn batch_cancel_structured_timeout_without_read_falls_back() {
    let mut params = base_params();
    params.timeout = Some(TimeoutValue::Structured(StructuredTimeout::default()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    drive(&service, &request("vertex_ai", params, CancelCall::Sync)).expect("answered");
    assert_eq!(recorded(&calls)[0]["timeout"], json!(600.0));
}

#[test]
fn batch_cancel_rejects_a_timeout_that_is_not_a_number() {
    let mut params = base_params();
    params.timeout = Some(TimeoutValue::Text("later".to_owned()));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let error = drive(&service, &request("openai", params, CancelCall::Sync))
        .expect_err("the timeout is not a number");
    assert!(error.to_string().contains("later"), "{error}");
    assert!(recorded(&calls).is_empty());
}

#[rstest]
#[case::sync_uses_the_argument(CancelCall::Sync, Some("gpt-4o"), Some("gpt-4o"), false)]
#[case::async_keeps_the_argument(CancelCall::Async { model_in_kwargs: None }, Some("gpt-4o"), Some("gpt-4o"), true)]
#[case::async_kwargs_model_wins(CancelCall::Async { model_in_kwargs: Some("gpt-4o-mini".to_owned()) }, Some("gpt-4o"), Some("gpt-4o-mini"), true)]
#[case::async_without_a_model(CancelCall::Async { model_in_kwargs: None }, None, None, true)]
fn batch_cancel_merges_the_model(
    #[case] call: CancelCall,
    #[case] model: Option<&str>,
    #[case] expected: Option<&str>,
    #[case] is_async: bool,
) {
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: model.map(str::to_owned),
        provider: CancelProvider::Openai,
        extra_headers: None,
        extra_body: None,
        params: base_params(),
        call,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let result = drive(&service, &input).expect("answered");
    assert_eq!(result.model.as_deref(), expected);
    assert_eq!(recorded(&calls)[0]["is_async"], json!(is_async));
}

#[test]
fn batch_cancel_rejects_an_unsupported_provider_with_400() {
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let error = drive(
        &service,
        &request("mistral", base_params(), CancelCall::Sync),
    )
    .expect_err("the provider is unsupported");
    let fields = error.error_fields().expect("an HTTP error");
    assert_eq!(fields.status, 400);
    assert_eq!(fields.error_type, "bad_request_error");
    assert_eq!(fields.code, "400");
    assert!(error.to_string().contains("mistral"), "{error}");
    assert!(recorded(&calls).is_empty());
}

#[rstest]
#[case::cancelling(BatchStatus::Cancelling)]
#[case::cancelled(BatchStatus::Cancelled)]
#[case::completed(BatchStatus::Completed)]
fn batch_cancel_preserves_the_provider_status(#[case] status: BatchStatus) {
    let (dispatch, _calls) = scripted(status);
    let service = service(base_settings(), dispatch);
    let result = drive(
        &service,
        &request("openai", base_params(), CancelCall::Sync),
    )
    .expect("answered");
    assert_eq!(result.batch.status, status);
}

#[rstest]
#[case::at_the_limit(Some(("a", "bc")), None, false)]
#[case::headers_over_the_limit(Some(("ab", "cd")), None, true)]
#[case::body_over_the_limit(None, Some(("ab", "cd")), true)]
#[case::both_over_the_limit(Some(("a", "bc")), Some(("d", "e")), true)]
fn batch_cancel_bounds_the_extra_bytes(
    #[case] header: Option<(&str, &str)>,
    #[case] body: Option<(&str, &str)>,
    #[case] rejected: bool,
) {
    let defaults = ApiLimits::default();
    let limits = ApiLimits {
        max_json_bytes: 10,
        max_event_bytes: defaults.max_event_bytes,
        max_items: defaults.max_items,
    };
    let entry = |pair: (&str, &str)| BTreeMap::from([(pair.0.to_owned(), pair.1.to_owned())]);
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider::Openai,
        extra_headers: header.map(entry),
        extra_body: body.map(entry),
        params: base_params(),
        call: CancelCall::Sync,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = BatchProviderCancel {
        clock: Arc::new(|| SECOND),
        dispatch,
        limits,
        settings: base_settings(),
    };
    let outcome = drive(&service, &input);
    assert_eq!(
        matches!(outcome, Err(BatchProviderCancelError::Limits(_))),
        rejected
    );
    assert_eq!(recorded(&calls).is_empty(), rejected);
}

#[test]
fn batch_cancel_bounds_the_extra_entries() {
    let limit = ApiLimits::default().max_items;
    let extra_headers: BTreeMap<String, String> = (0..=limit)
        .map(|index| (index.to_string(), String::new()))
        .collect();
    let input = BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider::Openai,
        extra_headers: Some(extra_headers),
        extra_body: None,
        params: base_params(),
        call: CancelCall::Sync,
    }));
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(base_settings(), dispatch);
    let error = drive(&service, &input).expect_err("too many entries");
    assert!(matches!(error, BatchProviderCancelError::Limits(_)));
    assert!(recorded(&calls).is_empty());
}

fn status_response(raw: &str) -> AsyncInvokeStatus {
    AsyncInvokeStatus {
        invocation_arn: "arn:aws:bedrock:us-east-1:1:async-invoke/abc".to_owned(),
        model_arn: "arn:aws:bedrock:us-east-1::foundation-model/marengo".to_owned(),
        status: raw.to_owned(),
        failure_message: None,
        output_data_config: None,
        submit_time: Some("2024-01-01T00:00:00Z".to_owned()),
        last_modified_time: Some("2024-01-01T00:01:00Z".to_owned()),
        end_time: Some("2024-01-01T00:02:00Z".to_owned()),
    }
}

fn async_result(status: &AsyncInvokeStatus) -> BatchProviderCancelResult {
    async_invoke::normalize(status, SECOND, &ApiLimits::default()).expect("normalized")
}

#[rstest]
#[case::completed("Completed", BatchStatus::Completed, 1, 0)]
#[case::lowercase_completed("completed", BatchStatus::Completed, 1, 0)]
#[case::uppercase_completed("COMPLETED", BatchStatus::Completed, 1, 0)]
#[case::in_progress("InProgress", BatchStatus::InProgress, 0, 0)]
#[case::in_progress_snake("in_progress", BatchStatus::InProgress, 0, 0)]
#[case::validating("Validating", BatchStatus::Failed, 0, 1)]
fn async_invoke_completed(
    #[case] raw: &str,
    #[case] expected: BatchStatus,
    #[case] completed: i64,
    #[case] failed: i64,
) {
    let result = async_result(&status_response(raw));
    let batch = result.batch;
    assert_eq!(batch.status, expected);
    assert_eq!(batch.object, BatchObjectKind::Batch);
    assert_eq!(batch.endpoint, "/v1/embeddings");
    assert_eq!(batch.completion_window, "24h");
    assert_eq!(batch.input_file_id, "");
    assert_eq!(batch.created_at, SECOND);
    assert_eq!(batch.id, "arn:aws:bedrock:us-east-1:1:async-invoke/abc");
    let counts = batch.request_counts.expect("counts");
    assert_eq!(counts.total, 1);
    assert_eq!(counts.completed, completed);
    assert_eq!(counts.failed, failed);
    assert_eq!(result.model, None);
}

#[rstest]
#[case::failed("Failed", 1)]
#[case::lowercase_failed("failed", 1)]
#[case::stopped("Stopped", 1)]
#[case::partially_completed("PartiallyCompleted", 1)]
#[case::unknown("Exploded", 1)]
#[case::empty("", 1)]
fn async_invoke_failed(#[case] raw: &str, #[case] failed: i64) {
    let mut status = status_response(raw);
    status.failure_message = Some("the job stopped".to_owned());
    let batch = async_result(&status).batch;
    assert_eq!(batch.status, BatchStatus::Failed);
    let counts = batch.request_counts.expect("counts");
    assert_eq!(counts.failed, failed);
    assert_eq!(counts.completed, 0);
    let metadata = batch.metadata.expect("metadata");
    assert_eq!(
        metadata.get("failure_message").map(String::as_str),
        Some("the job stopped")
    );
}

#[rstest]
#[case::completed_stamps_the_end("Completed", Some(SECOND + 120), None, None)]
#[case::failed_stamps_the_failure("Failed", None, Some(SECOND + 120), None)]
#[case::in_progress_stamps_the_change("InProgress", None, None, Some(SECOND + 60))]
#[case::lowercase_completed_keeps_no_end("completed", None, None, None)]
#[case::partially_completed_stamps_completion("PartiallyCompleted", Some(SECOND + 120), None, None)]
fn async_invoke_timestamps(
    #[case] raw: &str,
    #[case] completed_at: Option<i64>,
    #[case] failed_at: Option<i64>,
    #[case] in_progress_at: Option<i64>,
) {
    let batch = async_result(&status_response(raw)).batch;
    assert_eq!(batch.completed_at, completed_at);
    assert_eq!(batch.failed_at, failed_at);
    assert_eq!(batch.in_progress_at, in_progress_at);
}

#[rstest]
#[case::s3_uri(
    Some(AsyncInvokeOutput {
        s3_output_data_config: Some(AsyncInvokeS3Output {
            s3_uri: Some("s3://bucket/output.jsonl".to_owned()),
        }),
    }),
    "s3://bucket/output.jsonl"
)]
#[case::missing_output_config(None, "")]
#[case::missing_s3_config(
    Some(AsyncInvokeOutput {
        s3_output_data_config: None,
    }),
    ""
)]
#[case::missing_s3_uri(
    Some(AsyncInvokeOutput {
        s3_output_data_config: Some(AsyncInvokeS3Output { s3_uri: None }),
    }),
    ""
)]
fn async_invoke_output_location(
    #[case] output_data_config: Option<AsyncInvokeOutput>,
    #[case] expected: &str,
) {
    let mut status = status_response("Completed");
    status.output_data_config = output_data_config;
    let batch = async_result(&status).batch;
    let metadata = batch.metadata.expect("metadata");
    assert_eq!(
        metadata.get("output_file_id").map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        metadata.get("model_arn").map(String::as_str),
        Some("arn:aws:bedrock:us-east-1::foundation-model/marengo")
    );
    assert_eq!(batch.output_file_id, None);
}

#[test]
fn async_invoke_without_a_submit_time_uses_the_clock() {
    let mut status = status_response("Completed");
    status.submit_time = None;
    let batch = async_invoke::normalize(&status, SECOND, &ApiLimits::default())
        .expect("normalized")
        .batch;
    assert_eq!(batch.created_at, SECOND);
}

#[test]
fn async_invoke_rejects_an_oversized_failure_message() {
    let mut status = status_response("Failed");
    status.failure_message =
        Some("x".repeat(ApiLimits::default().max_event_bytes.saturating_add(1)));
    let error = async_invoke::normalize(&status, SECOND, &ApiLimits::default())
        .expect_err("the failure message is too large");
    assert_eq!(error.fields().code, "event_too_large");
}

#[test]
fn connection_defaults_match_litellm() {
    assert_eq!(connection::DEFAULT_TIMEOUT_SECONDS, 600.0);
    assert_eq!(connection::DEFAULT_OPENAI_BASE, "https://api.openai.com/v1");
    assert!(connection::supports_httpx_timeout(&CancelProvider::Openai));
    assert!(connection::supports_httpx_timeout(&CancelProvider::Azure));
    assert!(connection::supports_httpx_timeout(&CancelProvider::Bedrock));
    assert!(!connection::supports_httpx_timeout(
        &CancelProvider::VertexAi
    ));
    assert!(!connection::supports_httpx_timeout(&CancelProvider::Xai));
    assert!(!connection::supports_httpx_timeout(
        &CancelProvider::HostedVllm
    ));
}

#[test]
fn cancel_provider_reads_its_wire_name() {
    let provider = CancelProvider::from("hosted_vllm".to_owned());
    assert_eq!(provider, CancelProvider::HostedVllm);
    let decoded: CancelProvider = serde_json::from_value(json!("xai")).expect("decode");
    assert_eq!(decoded, CancelProvider::Xai);
    let other = CancelProvider::from_name("mistral");
    assert_eq!(other, CancelProvider::Other("mistral".to_owned()));
}

#[test]
fn batch_provider_cancel_input_round_trips_through_json() {
    let input = request("openai", base_params(), CancelCall::Sync);
    let encoded = serde_json::to_value(&input).expect("encode");
    let decoded: BatchProviderCancelInput =
        serde_json::from_value(encoded.clone()).expect("decode");
    assert_eq!(serde_json::to_value(&decoded).expect("re-encode"), encoded);
}

#[test]
fn cancel_timeout_serializes_as_a_number() {
    let timeout = CancelTimeout::Seconds(12.0);
    assert_eq!(serde_json::to_value(timeout).expect("json"), json!(12.0));
}
