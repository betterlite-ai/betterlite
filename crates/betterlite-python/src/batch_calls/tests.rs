#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::xai_identity;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::batch_provider_create::error::BatchProviderCreateError;
use betterlite_core::api::batch_provider_create::transports::BatchCreateTransport;
use betterlite_core::api::batch_provider_create::{
    BatchProvider, BatchProviderCreateCall, BatchProviderCreateInput, ProviderBatchCall,
    ProviderConfigCall,
};
use betterlite_core::api::batch_provider_read::call::{
    AnthropicBatchReadCall, BatchProviderReadCall,
};
use betterlite_core::api::batch_provider_read::{BatchProviderReadInput, BatchReadDispatch};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::batch_records::{CreateBatchRequest, WireDecodeLimits};
use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::batches::sdk::Timeout;
use betterlite_openai::xai_batches::error::XaiBatchesError;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;
use serde::Deserialize;

use super::convert::{CallMode, HiddenParams, parse};
use super::error::{DispatchFailure, payload_for};
use super::register;
use super::session::BatchCore;
use super::transport::{BatchDispatch, BatchTransport, create_status, sdk_status, xai_status};

const BATCH: &str = r#"{"id":"batch-1","completion_window":"24h","created_at":0,"endpoint":"/v1/chat/completions","input_file_id":"file-1","object":"batch","status":"validating"}"#;
const CREATE: &str = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"file-1","custom_llm_provider":"openai","model":"gpt-4o"}"#;
const READ: &str = r#"{"batch_id":"batch-1","provider":"openai"}"#;
const EMPTY_ENVIRONMENT: &str = r#"{"environment":{}}"#;
// no listener answers port 1, so a session's provider call fails without any network behind it
const CREATE_REFUSED: &str = r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"file-1","custom_llm_provider":"openai","model":"gpt-4o","optional_params":{"api_base":"http://127.0.0.1:1/v1","api_key":"sk-test"}}"#;
const READ_REFUSED: &str = r#"{"batch_id":"batch-1","provider":"openai","model":"gpt-4o","optional_params":{"api_base":"http://127.0.0.1:1/v1","api_key":"sk-test"}}"#;

#[derive(Debug, Deserialize)]
struct AnswerView {
    id: String,
    #[serde(rename = "_hidden_params")]
    hidden_params: HiddenParams,
}

struct Answers(BatchObject);

impl BatchCreateTransport for Answers {
    fn create(
        &self,
        _call: &BatchProviderCreateCall,
    ) -> Result<BatchObject, BatchProviderCreateError> {
        Ok(self.0.clone())
    }
}

struct AnswersRead(BatchObject);

impl BatchReadDispatch for AnswersRead {
    type Error = DispatchFailure;

    fn retrieve(
        &self,
        _call: &BatchProviderReadCall,
    ) -> impl Future<Output = Result<BatchObject, DispatchFailure>> + Send {
        let batch = self.0.clone();
        async move { Ok(batch) }
    }
}

struct RefusesRead {
    status: u16,
    message: String,
}

impl BatchReadDispatch for RefusesRead {
    type Error = DispatchFailure;

    fn retrieve(
        &self,
        _call: &BatchProviderReadCall,
    ) -> impl Future<Output = Result<BatchObject, DispatchFailure>> + Send {
        let status = self.status;
        let message = self.message.clone();
        async move { Err(DispatchFailure::Status { status, message }) }
    }
}

fn core() -> BatchCore {
    BatchCore::build(EMPTY_ENVIRONMENT).unwrap()
}

fn batch() -> BatchObject {
    parse(BATCH).unwrap()
}

fn proxied() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn decode_limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1024,
        max_event_bytes: 1024,
        max_items: 16,
    }
}

fn transport() -> BatchTransport {
    BatchTransport {
        client: proxied(),
        xai: xai_identity::Settings::default(),
        limits: decode_limits(),
    }
}

fn dispatch() -> BatchDispatch {
    BatchDispatch {
        client: proxied(),
        xai: xai_identity::Settings::default(),
        limits: decode_limits(),
    }
}

fn openai_call() -> ProviderBatchCall {
    ProviderBatchCall {
        body: CreateBatchRequest::default(),
        api_base: None,
        api_key: None,
        organization: None,
        api_version: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        custom_endpoint: None,
        timeout: 600.0,
        max_retries: None,
        is_async: false,
    }
}

#[test]
fn create_with_answers_the_batch_and_hidden_params() {
    let input: BatchProviderCreateInput = parse(CREATE).unwrap();
    let answer = core()
        .create_with(input, CallMode::Sync, Answers(batch()))
        .unwrap();
    let view: AnswerView = parse(&answer).unwrap();
    assert_eq!(view.id, "batch-1");
    assert_eq!(view.hidden_params.llm_provider, "openai");
    assert_eq!(view.hidden_params.model_id, None);
    assert_eq!(
        view.hidden_params.litellm_model_name.as_deref(),
        Some("gpt-4o")
    );
    assert_eq!(
        view.hidden_params.api_base.as_deref(),
        Some("https://api.openai.com/v1")
    );
}

#[test]
fn create_with_marks_an_async_call() {
    let input: BatchProviderCreateInput = parse(CREATE).unwrap();
    let answer = core()
        .create_with(input, CallMode::Async, Answers(batch()))
        .unwrap();
    let view: AnswerView = parse(&answer).unwrap();
    assert_eq!(view.id, "batch-1");
}

#[test]
fn create_with_reports_an_unsupported_provider() {
    let input: BatchProviderCreateInput = parse(
        r#"{"completion_window":"24h","endpoint":"/v1/chat/completions","input_file_id":"file-1","custom_llm_provider":"bedrock"}"#,
    )
    .unwrap();
    let error = core()
        .create_with(input, CallMode::Sync, Answers(batch()))
        .unwrap_err();
    let payload = payload_for(&error, "bedrock", None).unwrap();
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert_eq!(payload.llm_provider, "bedrock");
    assert_eq!(payload.model.as_deref(), Some("n/a"));
    assert!(payload.message.contains("bedrock"));
}

#[test]
fn retrieve_with_answers_the_batch_and_hidden_params() {
    let input: BatchProviderReadInput = parse(READ).unwrap();
    let answer = get_runtime()
        .block_on(core().retrieve_with(input, CallMode::Sync, AnswersRead(batch())))
        .unwrap();
    let view: AnswerView = parse(&answer).unwrap();
    assert_eq!(view.id, "batch-1");
    assert_eq!(view.hidden_params.llm_provider, "openai");
    assert_eq!(view.hidden_params.model_id, None);
    assert_eq!(view.hidden_params.litellm_model_name, None);
    assert_eq!(
        view.hidden_params.api_base.as_deref(),
        Some("https://api.openai.com/v1")
    );
}

#[test]
fn retrieve_with_reports_a_provider_status() {
    let input: BatchProviderReadInput = parse(READ).unwrap();
    let dispatch = RefusesRead {
        status: 404,
        message: "Error code: 404 - no such batch".to_owned(),
    };
    let error = get_runtime()
        .block_on(core().retrieve_with(input, CallMode::Sync, dispatch))
        .unwrap_err();
    let payload = payload_for(&error, "openai", Some("gpt-4o")).unwrap();
    assert_eq!(payload.error_type, "NotFoundError");
    assert_eq!(payload.status_code, Some(404));
    assert_eq!(payload.message, "Error code: 404 - no such batch");
    assert_eq!(payload.llm_provider, "openai");
    assert_eq!(payload.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn retrieve_with_refuses_an_unsupported_provider() {
    let input: BatchProviderReadInput =
        parse(r#"{"batch_id":"batch-1","provider":"cohere"}"#).unwrap();
    let error = get_runtime()
        .block_on(core().retrieve_with(input, CallMode::Sync, AnswersRead(batch())))
        .unwrap_err();
    let payload = payload_for(&error, "cohere", None).unwrap();
    assert_eq!(payload.error_type, "BadRequestError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.message.contains("cohere"));
}

#[test]
fn transport_refuses_an_unwired_create_provider() {
    let transport = transport();
    let azure = BatchProviderCreateCall::Azure(openai_call());
    match transport.create(&azure) {
        Err(BatchProviderCreateError::UnsupportedProvider { provider }) => {
            assert_eq!(provider, BatchProvider::Azure);
        }
        Ok(_) | Err(_other) => panic!("the azure branch is not wired in this build"),
    }
    let config = BatchProviderCreateCall::ProviderConfig(ProviderConfigCall {
        provider: BatchProvider::Bedrock,
        model: "bedrock/model".to_owned(),
        body: CreateBatchRequest::default(),
        headers: BTreeMap::new(),
        api_base: None,
        api_key: None,
        timeout: 600.0,
        is_async: false,
    });
    match transport.create(&config) {
        Err(BatchProviderCreateError::UnsupportedProvider { provider }) => {
            assert_eq!(provider, BatchProvider::Bedrock);
        }
        Ok(_) | Err(_other) => panic!("the provider config branch is not wired in this build"),
    }
}

#[test]
fn transport_reports_a_bad_openai_body() {
    let outcome = transport().create(&BatchProviderCreateCall::OpenAiCompatible(openai_call()));
    assert!(matches!(
        outcome,
        Err(BatchProviderCreateError::Provider { status: 400 })
    ));
}

#[test]
fn transport_reports_a_bad_xai_timeout() {
    let mut call = openai_call();
    call.timeout = -1.0;
    let outcome = transport().create(&BatchProviderCreateCall::Xai(call));
    assert!(matches!(
        outcome,
        Err(BatchProviderCreateError::Provider { status: 400 })
    ));
}

#[test]
fn dispatch_refuses_an_unwired_read_provider() {
    let call = BatchProviderReadCall::Anthropic(Box::new(AnthropicBatchReadCall {
        batch_id: "batch-1".to_owned(),
        api_base: None,
        api_key: None,
        timeout: Timeout::Seconds(600.0),
        max_retries: None,
        is_async: false,
    }));
    let outcome = get_runtime().block_on(dispatch().retrieve(&call));
    match outcome {
        Err(DispatchFailure::Unsupported { provider }) => {
            assert_eq!(provider, "anthropic");
        }
        Ok(_) | Err(_other) => panic!("the anthropic branch is not wired in this build"),
    }
}

#[test]
fn transport_statuses_follow_the_sdk_classes() {
    let status = SdkError::Status {
        status: 429,
        error_type: None,
        param: None,
        code: None,
        message: "slow down".to_owned(),
    };
    assert_eq!(sdk_status(&status), 429);
    assert_eq!(create_status(&BatchesError::NotInitialized), 401);
    assert_eq!(create_status(&BatchesError::NotAsync), 500);
    assert_eq!(xai_status(&XaiBatchesError::MissingApiKey), 401);
    assert_eq!(xai_status(&XaiBatchesError::InputFileRequired), 400);
}

#[test]
fn register_adds_the_batch_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeBatchSession").unwrap());
        assert!(module.hasattr("NativeBatchError").unwrap());
    });
}

// every sync session method runs its call and hands back the provider and model the payload asked for
#[test]
fn session_errors_name_the_provider_and_model_the_payload_asked_for() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        let class = module.getattr("NativeBatchSession").unwrap();
        let session = class.call1((EMPTY_ENVIRONMENT,)).unwrap();
        for (method, payload) in [("create", CREATE_REFUSED), ("retrieve", READ_REFUSED)] {
            let error = session
                .call_method1(method, (payload, Vec::<Vec<u8>>::new()))
                .unwrap_err();
            let message = error.value(py).to_string();
            assert!(message.starts_with(r#"{"type":"#), "{method}: {message}");
            assert!(
                message.contains(r#""llm_provider":"openai""#),
                "{method}: {message}"
            );
            assert!(
                message.contains(r#""model":"gpt-4o""#),
                "{method}: {message}"
            );
        }
    });
}

#[test]
fn dispatch_failures_keep_the_provider_status() {
    let error = BatchesError::Sdk {
        source: SdkError::Status {
            status: 400,
            error_type: None,
            param: None,
            code: None,
            message: "bad".to_owned(),
        },
    };
    match DispatchFailure::from(error) {
        DispatchFailure::Status { status, message } => {
            assert_eq!(status, 400);
            assert_eq!(message, "bad");
        }
        other => panic!("unexpected failure: {other:?}"),
    }
    let connection = DispatchFailure::from(BatchesError::NotInitialized);
    assert!(matches!(connection, DispatchFailure::Connection(_)));
}
