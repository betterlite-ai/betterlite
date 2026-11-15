#![cfg(test)]

mod call;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::batch_provider_read::Settings;
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use betterlite_openai::batches::sdk::Timeout;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use super::call::{
    AnthropicBatchReadCall, AwsCredentials, AwsSessionTag, AzureBatchReadCall, AzureReadParams,
    BatchProviderReadCall, OpenAiBatchReadCall, ProviderConfigReadCall, VertexBatchReadCall,
    VertexReadCredentials, XaiBatchReadCall,
};
use super::error::BatchProviderReadError;
use super::{
    BatchProviderRead, BatchProviderReadInput, BatchProviderReadResult, BatchReadDispatch,
    BatchReadOptionalParams, BatchReadProvider, BatchReadTimeoutValue,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::resource_sdk_settings;

#[derive(Debug, Clone, PartialEq)]
struct RecordedAws {
    region_name: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    session_name: Option<String>,
    profile_name: Option<String>,
    role_name: Option<String>,
    web_identity_token: Option<String>,
    sts_endpoint: Option<String>,
    external_id: Option<String>,
    session_tags: Option<Vec<AwsSessionTag>>,
    bedrock_runtime_endpoint: Option<String>,
    bedrock_project_id: Option<String>,
}

fn recorded_aws(aws: &AwsCredentials) -> RecordedAws {
    RecordedAws {
        region_name: aws.region_name.clone(),
        access_key_id: aws.access_key_id.clone(),
        secret_access_key: aws.secret_access_key.as_ref().map(secret_text),
        session_token: aws.session_token.as_ref().map(secret_text),
        session_name: aws.session_name.clone(),
        profile_name: aws.profile_name.clone(),
        role_name: aws.role_name.clone(),
        web_identity_token: aws.web_identity_token.as_ref().map(secret_text),
        sts_endpoint: aws.sts_endpoint.clone(),
        external_id: aws.external_id.clone(),
        session_tags: aws.session_tags.clone(),
        bedrock_runtime_endpoint: aws.bedrock_runtime_endpoint.clone(),
        bedrock_project_id: aws.bedrock_project_id.clone(),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedParams {
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
    max_retries: Option<i64>,
    timeout: Option<BatchReadTimeoutValue>,
    azure_ad_token: Option<String>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    azure_scope: Option<String>,
    azure_username: Option<String>,
    azure_password: Option<String>,
}

fn recorded_params(params: &AzureReadParams) -> RecordedParams {
    RecordedParams {
        api_key: params.api_key.as_ref().map(secret_text),
        api_base: params.api_base.clone(),
        api_version: params.api_version.clone(),
        max_retries: params.max_retries,
        timeout: params.timeout.clone(),
        azure_ad_token: params.azure_ad_token.as_ref().map(secret_text),
        tenant_id: params.tenant_id.clone(),
        client_id: params.client_id.clone(),
        client_secret: params.client_secret.as_ref().map(secret_text),
        azure_scope: params.azure_scope.clone(),
        azure_username: params.azure_username.clone(),
        azure_password: params.azure_password.as_ref().map(secret_text),
    }
}

// what a provider branch received, owned: secrets exposed so a value can be asserted
#[derive(Debug, Clone, PartialEq)]
enum Recorded {
    ProviderConfig {
        provider: BatchReadProvider,
        model: String,
        batch_id: String,
        headers: BTreeMap<String, String>,
        api_base: Option<String>,
        api_key: Option<String>,
        aws: RecordedAws,
        timeout: Timeout,
        is_async: bool,
    },
    Xai {
        batch_id: String,
        api_base: Option<String>,
        api_key: Option<String>,
        timeout: Timeout,
        is_async: bool,
    },
    OpenAi {
        request: RetrieveBatchRequest,
        api_base: String,
        api_key: Option<String>,
        organization: Option<String>,
        timeout: Timeout,
        max_retries: Option<i64>,
        is_async: bool,
    },
    Azure {
        request: RetrieveBatchRequest,
        api_base: Option<String>,
        api_key: Option<String>,
        api_version: Option<String>,
        timeout: Timeout,
        max_retries: Option<i64>,
        litellm_params: RecordedParams,
        is_async: bool,
    },
    VertexAi {
        batch_id: String,
        api_base: String,
        vertex_project: Option<String>,
        vertex_location: Option<String>,
        vertex_credentials: Option<VertexReadCredentials>,
        timeout: Timeout,
        max_retries: Option<i64>,
        is_async: bool,
    },
    Anthropic {
        batch_id: String,
        api_base: Option<String>,
        api_key: Option<String>,
        timeout: Timeout,
        max_retries: Option<i64>,
        is_async: bool,
    },
    BedrockAsyncInvoke {
        batch_id: String,
        aws: RecordedAws,
    },
    BedrockModelInvocationJob {
        batch_id: String,
        aws: RecordedAws,
    },
}

fn secret_text(secret: &SecretString) -> String {
    secret.expose_secret().to_owned()
}

fn recorded(call: &BatchProviderReadCall) -> Recorded {
    match call {
        BatchProviderReadCall::ProviderConfig(call) => provider_config_record(call),
        BatchProviderReadCall::Xai(call) => xai_record(call),
        BatchProviderReadCall::OpenAiCompatible(call) => openai_record(call),
        BatchProviderReadCall::Azure(call) => azure_record(call),
        BatchProviderReadCall::VertexAi(call) => vertex_record(call),
        BatchProviderReadCall::Anthropic(call) => anthropic_record(call),
        BatchProviderReadCall::BedrockAsyncInvoke(call) => Recorded::BedrockAsyncInvoke {
            batch_id: call.batch_id.clone(),
            aws: recorded_aws(&call.aws),
        },
        BatchProviderReadCall::BedrockModelInvocationJob(call) => {
            Recorded::BedrockModelInvocationJob {
                batch_id: call.batch_id.clone(),
                aws: recorded_aws(&call.aws),
            }
        }
    }
}

fn xai_record(call: &XaiBatchReadCall) -> Recorded {
    Recorded::Xai {
        batch_id: call.batch_id.clone(),
        api_base: call.api_base.clone(),
        api_key: call.api_key.as_ref().map(secret_text),
        timeout: call.timeout.clone(),
        is_async: call.is_async,
    }
}

fn openai_record(call: &OpenAiBatchReadCall) -> Recorded {
    Recorded::OpenAi {
        request: call.request.clone(),
        api_base: call.api_base.clone(),
        api_key: call.api_key.as_ref().map(secret_text),
        organization: call.organization.clone(),
        timeout: call.timeout.clone(),
        max_retries: call.max_retries,
        is_async: call.is_async,
    }
}

fn azure_record(call: &AzureBatchReadCall) -> Recorded {
    Recorded::Azure {
        request: call.request.clone(),
        api_base: call.api_base.clone(),
        api_key: call.api_key.as_ref().map(secret_text),
        api_version: call.api_version.clone(),
        timeout: call.timeout.clone(),
        max_retries: call.max_retries,
        litellm_params: recorded_params(&call.litellm_params),
        is_async: call.is_async,
    }
}

fn vertex_record(call: &VertexBatchReadCall) -> Recorded {
    Recorded::VertexAi {
        batch_id: call.batch_id.clone(),
        api_base: call.api_base.clone(),
        vertex_project: call.vertex_project.clone(),
        vertex_location: call.vertex_location.clone(),
        vertex_credentials: call.vertex_credentials.clone(),
        timeout: call.timeout.clone(),
        max_retries: call.max_retries,
        is_async: call.is_async,
    }
}

fn anthropic_record(call: &AnthropicBatchReadCall) -> Recorded {
    Recorded::Anthropic {
        batch_id: call.batch_id.clone(),
        api_base: call.api_base.clone(),
        api_key: call.api_key.as_ref().map(secret_text),
        timeout: call.timeout.clone(),
        max_retries: call.max_retries,
        is_async: call.is_async,
    }
}

fn provider_config_record(call: &ProviderConfigReadCall) -> Recorded {
    Recorded::ProviderConfig {
        provider: call.provider.clone(),
        model: call.model.clone(),
        batch_id: call.batch_id.clone(),
        headers: call.headers.clone(),
        api_base: call.api_base.clone(),
        api_key: call.api_key.as_ref().map(secret_text),
        aws: recorded_aws(&call.aws),
        timeout: call.timeout.clone(),
        is_async: call.is_async,
    }
}

fn branch(entry: &Recorded) -> &'static str {
    match entry {
        Recorded::ProviderConfig { .. } => "provider_config",
        Recorded::Xai { .. } => "xai",
        Recorded::OpenAi { .. } => "openai",
        Recorded::Azure { .. } => "azure",
        Recorded::VertexAi { .. } => "vertex_ai",
        Recorded::Anthropic { .. } => "anthropic",
        Recorded::BedrockAsyncInvoke { .. } => "bedrock_async_invoke",
        Recorded::BedrockModelInvocationJob { .. } => "bedrock_model_invocation_job",
    }
}

fn base_settings() -> Settings {
    Settings::default()
}

fn configured(resources: resource_sdk_settings::Settings) -> Settings {
    Settings {
        resources,
        anthropic_api_base: None,
        anthropic_base_url: None,
        anthropic_api_key: None,
    }
}

fn input(provider: &str) -> BatchProviderReadInput {
    BatchProviderReadInput {
        batch_id: "batch_1".to_owned(),
        provider: BatchReadProvider::from(provider.to_owned()),
        extra_headers: None,
        extra_body: None,
        model: None,
        aretrieve_batch: false,
        optional_params: BatchReadOptionalParams::default(),
    }
}

fn answer(status: BatchStatus) -> BatchObject {
    BatchObject {
        id: "batch_1".to_owned(),
        completion_window: "24h".to_owned(),
        created_at: 0,
        endpoint: "/v1/chat/completions".to_owned(),
        input_file_id: "file_1".to_owned(),
        object: BatchObjectKind::Batch,
        status,
        cancelled_at: None,
        cancelling_at: None,
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
        provider_fields: <_>::default(),
    }
}

#[derive(Debug, Clone, Error)]
#[error("the provider answered with status {status}")]
struct ProviderStatus {
    status: u16,
}

struct Scripted {
    calls: Arc<Mutex<Vec<Recorded>>>,
    reply: Result<BatchObject, ProviderStatus>,
}

impl BatchReadDispatch for Scripted {
    type Error = ProviderStatus;

    fn retrieve(
        &self,
        call: &BatchProviderReadCall,
    ) -> impl Future<Output = Result<BatchObject, ProviderStatus>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(call));
        let reply = self.reply.clone();
        async move { reply }
    }
}

fn scripted(reply: Result<BatchObject, ProviderStatus>) -> (Scripted, Arc<Mutex<Vec<Recorded>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dispatch = Scripted {
        calls: Arc::clone(&calls),
        reply,
    };
    (dispatch, calls)
}

fn service(settings: Settings, dispatch: Scripted) -> BatchProviderRead<Scripted> {
    BatchProviderRead {
        dispatch,
        limits: ApiLimits::default(),
        settings,
    }
}

fn recorded_calls(calls: &Arc<Mutex<Vec<Recorded>>>) -> Vec<Recorded> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

// drives the production service with a provider that answers the scripted batch
async fn answered(
    settings: Settings,
    input: &BatchProviderReadInput,
) -> (BatchProviderReadResult, Vec<Recorded>) {
    let (dispatch, calls) = scripted(Ok(answer(BatchStatus::Completed)));
    let service = service(settings, dispatch);
    let result = service.run(input).await.expect("the provider answered");
    (result, recorded_calls(&calls))
}

// drives the production service with a provider that fails the way its adapter would
async fn failed(
    settings: Settings,
    input: &BatchProviderReadInput,
    status: u16,
) -> (BatchProviderReadError<ProviderStatus>, Vec<Recorded>) {
    let (dispatch, calls) = scripted(Err(ProviderStatus { status }));
    let service = service(settings, dispatch);
    let error = service.run(input).await.expect_err("the provider failed");
    (error, recorded_calls(&calls))
}

fn request(batch_id: &str) -> RetrieveBatchRequest {
    RetrieveBatchRequest {
        batch_id: Some(batch_id.to_owned()),
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

// the three providers retrieve_batch routes through the OpenAI-compatible branch share one call shape
#[rstest]
#[case::openai("openai", false)]
#[case::hosted_vllm("hosted_vllm", false)]
#[case::litellm_proxy("litellm_proxy", false)]
#[case::openai_async("openai", true)]
#[tokio::test]
async fn batch_retrieve(#[case] provider: &str, #[case] aretrieve_batch: bool) {
    let mut call = input(provider);
    call.aretrieve_batch = aretrieve_batch;
    let (result, calls) = answered(base_settings(), &call).await;
    assert_eq!(
        calls,
        vec![Recorded::OpenAi {
            request: request("batch_1"),
            api_base: "https://api.openai.com/v1".to_owned(),
            api_key: None,
            organization: None,
            timeout: Timeout::Seconds(600.0),
            max_retries: None,
            is_async: aretrieve_batch,
        }]
    );
    assert_eq!(recorded(&result.call), calls[0]);
    assert_eq!(result.batch, answer(BatchStatus::Completed));
}

// each provider without a provider config reads its own subset of the shared settings
#[rstest]
#[case::xai("xai", Recorded::Xai {
    batch_id: "batch_1".to_owned(),
    api_base: None,
    api_key: None,
    timeout: Timeout::Seconds(600.0),
    is_async: false,
})]
#[case::azure("azure", Recorded::Azure {
    request: request("batch_1"),
    api_base: None,
    api_key: Some("sk-configured".to_owned()),
    api_version: None,
    timeout: Timeout::Seconds(600.0),
    max_retries: None,
    litellm_params: RecordedParams {
        api_key: None,
        api_base: None,
        api_version: None,
        max_retries: None,
        timeout: None,
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: None,
        azure_username: None,
        azure_password: None,
    },
    is_async: false,
})]
#[case::vertex_ai("vertex_ai", Recorded::VertexAi {
    batch_id: "batch_1".to_owned(),
    api_base: String::new(),
    vertex_project: None,
    vertex_location: None,
    vertex_credentials: None,
    timeout: Timeout::Seconds(600.0),
    max_retries: None,
    is_async: false,
})]
#[case::anthropic("anthropic", Recorded::Anthropic {
    batch_id: "batch_1".to_owned(),
    api_base: None,
    api_key: Some("sk-configured".to_owned()),
    timeout: Timeout::Seconds(600.0),
    max_retries: None,
    is_async: false,
})]
#[tokio::test]
async fn legacy_provider_retrieve(#[case] provider: &str, #[case] expected: Recorded) {
    let mut settings = base_settings();
    settings.resources.api_key = Some(SecretString::from("sk-configured".to_owned()));
    let (result, calls) = answered(settings, &input(provider)).await;
    assert_eq!(calls, vec![expected]);
    assert_eq!(recorded(&result.call), calls[0]);
    assert_eq!(result.batch, answer(BatchStatus::Completed));
}

// a provider's not-found answer stays its own: retrieve_batch only re-raises it
#[rstest]
#[case::openai("openai", "openai", 404)]
#[case::azure("azure", "azure", 404)]
#[case::vertex_ai("vertex_ai", "vertex_ai", 404)]
#[case::xai("xai", "xai", 500)]
#[case::anthropic("anthropic", "anthropic", 400)]
#[tokio::test]
async fn batch_not_found(#[case] provider: &str, #[case] expected: &str, #[case] status: u16) {
    let (error, calls) = failed(base_settings(), &input(provider), status).await;
    assert_eq!(calls.len(), 1, "the provider was asked");
    assert_eq!(branch(&calls[0]), expected);
    let BatchProviderReadError::Dispatch(failure) = &error else {
        panic!("the provider's failure stays the provider's");
    };
    assert_eq!(failure.status, status);
    assert_eq!(error.error_fields(), None);
}
