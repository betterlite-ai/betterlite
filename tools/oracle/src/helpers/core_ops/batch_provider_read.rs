use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::batch_provider_read::{Raw, RawSettings};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_read::call::{
    AnthropicBatchReadCall, AzureBatchReadCall, AzureReadParams, BatchProviderReadCall,
    OpenAiBatchReadCall, ProviderConfigReadCall, VertexBatchReadCall, XaiBatchReadCall,
};
use betterlite_core::api::batch_provider_read::error::BatchProviderReadError;
use betterlite_core::api::batch_provider_read::{
    BatchProviderRead, BatchProviderReadInput, BatchProviderReadResult, BatchReadDispatch,
    BatchReadTimeoutValue,
};
use betterlite_core::genai_setup::params::VertexCredentials;
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use betterlite_openai::batches::sdk::{Timeout, TimeoutPhases};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    request: BatchProviderReadInput,
    #[serde(default)]
    litellm_settings: RawSettings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    script: Script,
}

// the answer the case scripts for the provider, or the status it fails with
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Script {
    status: Option<u16>,
}

#[derive(Debug, Error)]
#[error("the scripted provider answered {status}")]
struct ScriptedStatus {
    status: u16,
}

struct Scripted {
    script: Script,
    batch: BatchObject,
}

impl BatchReadDispatch for Scripted {
    type Error = ScriptedStatus;

    fn retrieve(
        &self,
        _call: &BatchProviderReadCall,
    ) -> impl Future<Output = Result<BatchObject, ScriptedStatus>> + Send {
        let batch = self.batch.clone();
        let status = self.script.status;
        async move {
            match status {
                None => Ok(batch),
                Some(status) => Err(ScriptedStatus { status }),
            }
        }
    }
}

fn secret_value(value: Option<&SecretString>) -> Value {
    value.map_or(Value::Null, |text| {
        Value::String(text.expose_secret().to_owned())
    })
}

fn phases_value(phases: &TimeoutPhases) -> Value {
    json!({
        "connect": phases.connect,
        "read": phases.read,
        "write": phases.write,
        "pool": phases.pool,
    })
}

fn timeout_value(timeout: &Timeout) -> Value {
    match timeout {
        Timeout::Seconds(seconds) => json!(seconds),
        Timeout::Phases(phases) => phases_value(phases),
    }
}

fn raw_timeout_value(value: Option<&BatchReadTimeoutValue>) -> Value {
    match value {
        None => Value::Null,
        Some(BatchReadTimeoutValue::Seconds(seconds)) => json!(seconds),
        Some(BatchReadTimeoutValue::Text(text)) => json!(text),
        Some(BatchReadTimeoutValue::Phases(phases)) => phases_value(phases),
    }
}

fn request_value(request: &RetrieveBatchRequest) -> Value {
    json!({
        "batch_id": request.batch_id,
        "extra_headers": request.extra_headers,
        "extra_body": request.extra_body,
    })
}

fn params_value(params: &AzureReadParams) -> Value {
    json!({
        "api_key": secret_value(params.api_key.as_ref()),
        "api_base": params.api_base,
        "api_version": params.api_version,
        "max_retries": params.max_retries,
        "timeout": raw_timeout_value(params.timeout.as_ref()),
        "azure_ad_token": secret_value(params.azure_ad_token.as_ref()),
        "tenant_id": params.tenant_id,
        "client_id": params.client_id,
        "client_secret": secret_value(params.client_secret.as_ref()),
        "azure_scope": params.azure_scope,
        "azure_username": params.azure_username,
        "azure_password": secret_value(params.azure_password.as_ref()),
    })
}

fn credentials_value(credentials: Option<&VertexCredentials>) -> HelperResult {
    match credentials {
        None => Ok(Value::Null),
        Some(VertexCredentials::Path(path)) => Ok(Value::String(path.clone())),
        Some(VertexCredentials::ServiceAccount(account)) => Ok(serde_json::to_value(account)?),
    }
}

fn provider_config_value(call: &ProviderConfigReadCall) -> Value {
    json!({
        "provider": call.provider.name(), "is_async": call.is_async, "model": call.model,
        "batch_id": call.batch_id, "headers": call.headers, "api_base": call.api_base,
        "api_key": secret_value(call.api_key.as_ref()), "timeout": timeout_value(&call.timeout),
    })
}

fn arn_value(route: &str, batch_id: &str) -> Value {
    json!({ "provider": "bedrock", "route": route, "batch_id": batch_id })
}

fn call_value(result: &BatchProviderReadResult) -> HelperResult {
    Ok(match &result.call {
        BatchProviderReadCall::ProviderConfig(call) => provider_config_value(call),
        BatchProviderReadCall::BedrockAsyncInvoke(call) => {
            arn_value("async_invoke", &call.batch_id)
        }
        BatchProviderReadCall::BedrockModelInvocationJob(call) => {
            arn_value("model_invocation_job", &call.batch_id)
        }
        BatchProviderReadCall::Xai(call) => xai_value(call),
        BatchProviderReadCall::OpenAiCompatible(call) => openai_value(call),
        BatchProviderReadCall::Azure(call) => azure_value(call),
        BatchProviderReadCall::VertexAi(call) => vertex_value(call)?,
        BatchProviderReadCall::Anthropic(call) => anthropic_value(call),
    })
}

fn xai_value(call: &XaiBatchReadCall) -> Value {
    json!({
        "provider": "xai", "is_async": call.is_async, "batch_id": call.batch_id,
        "api_base": call.api_base, "api_key": secret_value(call.api_key.as_ref()),
        "timeout": timeout_value(&call.timeout),
    })
}

fn openai_value(call: &OpenAiBatchReadCall) -> Value {
    json!({
        "provider": "openai", "is_async": call.is_async, "request": request_value(&call.request),
        "api_base": call.api_base, "api_key": secret_value(call.api_key.as_ref()),
        "organization": call.organization, "timeout": timeout_value(&call.timeout),
        "max_retries": call.max_retries,
    })
}

fn azure_value(call: &AzureBatchReadCall) -> Value {
    json!({
        "provider": "azure", "is_async": call.is_async, "request": request_value(&call.request),
        "api_base": call.api_base, "api_key": secret_value(call.api_key.as_ref()),
        "api_version": call.api_version, "timeout": timeout_value(&call.timeout),
        "max_retries": call.max_retries, "litellm_params": params_value(&call.litellm_params),
    })
}

fn vertex_value(call: &VertexBatchReadCall) -> HelperResult {
    let credentials = credentials_value(call.vertex_credentials.as_ref())?;
    Ok(json!({
        "provider": "vertex_ai", "is_async": call.is_async, "batch_id": call.batch_id,
        "api_base": call.api_base, "vertex_project": call.vertex_project,
        "vertex_location": call.vertex_location, "vertex_credentials": credentials,
        "timeout": timeout_value(&call.timeout), "max_retries": call.max_retries,
    }))
}

fn anthropic_value(call: &AnthropicBatchReadCall) -> Value {
    json!({
        "provider": "anthropic", "is_async": call.is_async, "batch_id": call.batch_id,
        "api_base": call.api_base, "api_key": secret_value(call.api_key.as_ref()),
        "timeout": timeout_value(&call.timeout), "max_retries": call.max_retries,
    })
}

fn failure(error: &BatchProviderReadError<ScriptedStatus>) -> Box<HelperError> {
    let (class, status): (&str, Option<u16>) = match error {
        BatchProviderReadError::Unsupported { .. } => ("BadRequestError", Some(400)),
        BatchProviderReadError::InvalidVertexCredentials => ("BadRequestError", Some(400)),
        BatchProviderReadError::Timeout(_) | BatchProviderReadError::Limits(_) => {
            ("ValueError", None)
        }
        BatchProviderReadError::Dispatch(failure) => ("APIError", Some(failure.status)),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

// the provider answer the case scripts; the compared value is the call the service built
fn echoed_batch() -> BatchObject {
    BatchObject {
        id: "batch_1".to_owned(),
        completion_window: "24h".to_owned(),
        created_at: 0,
        endpoint: "/v1/chat/completions".to_owned(),
        input_file_id: "file_1".to_owned(),
        object: BatchObjectKind::Batch,
        status: BatchStatus::Completed,
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

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let raw = Raw {
            litellm_settings: input.litellm_settings.clone(),
        };
        let settings = raw.convert(&input.environment).or_config_error()?;
        let service = BatchProviderRead {
            dispatch: Scripted {
                script: input.script.clone(),
                batch: echoed_batch(),
            },
            limits: ApiLimits::default(),
            settings,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        match runtime.block_on(service.run(&input.request)) {
            Ok(result) => call_value(&result),
            Err(error) => Err(failure(&error)),
        }
    }
}
