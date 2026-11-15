use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::batch_provider_read::Raw;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_provider_read::call::{
    AnthropicBatchReadCall, AzureBatchReadCall, AzureReadParams, BatchProviderReadCall,
    OpenAiBatchReadCall, ProviderConfigReadCall, VertexReadCredentials, XaiBatchReadCall,
};
use betterlite_core::api::batch_provider_read::error::BatchProviderReadError;
use betterlite_core::api::batch_provider_read::{
    BatchProviderRead, BatchProviderReadInput, BatchProviderReadResult, BatchReadDispatch,
    BatchReadTimeoutValue,
};
use betterlite_openai::api::batch_records::RetrieveBatchRequest;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::batches::sdk::Timeout;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    request: BatchProviderReadInput,
    // the case's config keys go back through the production Raw parser, as the shared raw settings keep no Clone
    #[serde(default)]
    litellm_settings: BTreeMap<String, Value>,
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

fn timeout_value(timeout: &Timeout) -> Value {
    match timeout {
        Timeout::Seconds(seconds) => json!(seconds),
        Timeout::Phases(phases) => json!(phases),
    }
}

fn raw_timeout_value(value: Option<&BatchReadTimeoutValue>) -> Value {
    match value {
        None => Value::Null,
        Some(raw) => json!(raw),
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

fn credentials_value(credentials: Option<&VertexReadCredentials>) -> HelperResult {
    match credentials {
        None => Ok(Value::Null),
        Some(VertexReadCredentials::Path(path)) => {
            Ok(Value::String(path.expose_secret().to_owned()))
        }
        Some(VertexReadCredentials::ServiceAccount(account)) => Ok(serde_json::to_value(account)?),
    }
}

fn provider_config_value(call: &ProviderConfigReadCall) -> Value {
    json!({
        "provider": call.provider.name(), "is_async": call.is_async, "model": call.model,
        "batch_id": call.batch_id, "headers": call.headers, "api_base": call.api_base,
        "api_key": secret_value(call.api_key.as_ref()), "timeout": timeout_value(&call.timeout),
    })
}

fn call_value(result: &BatchProviderReadResult) -> HelperResult {
    Ok(match &result.call {
        BatchProviderReadCall::ProviderConfig(call) => provider_config_value(call),
        BatchProviderReadCall::BedrockAsyncInvoke(call) => json!({
            "provider": "bedrock",
            "route": "async_invoke",
            "batch_id": call.batch_id,
        }),
        BatchProviderReadCall::BedrockModelInvocationJob(call) => json!({
            "provider": "bedrock",
            "route": "model_invocation_job",
            "batch_id": call.batch_id,
        }),
        BatchProviderReadCall::Xai(call) => xai_value(call),
        BatchProviderReadCall::OpenAiCompatible(call) => openai_value(call),
        BatchProviderReadCall::Azure(call) => azure_value(call),
        BatchProviderReadCall::VertexAi(call) => json!({
            "provider": "vertex_ai", "is_async": call.is_async, "batch_id": call.batch_id,
            "api_base": call.api_base, "vertex_project": call.vertex_project,
            "vertex_location": call.vertex_location,
            "vertex_credentials": credentials_value(call.vertex_credentials.as_ref())?,
            "timeout": timeout_value(&call.timeout), "max_retries": call.max_retries,
        }),
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

fn anthropic_value(call: &AnthropicBatchReadCall) -> Value {
    json!({
        "provider": "anthropic", "is_async": call.is_async, "batch_id": call.batch_id,
        "api_base": call.api_base, "api_key": secret_value(call.api_key.as_ref()),
        "timeout": timeout_value(&call.timeout), "max_retries": call.max_retries,
    })
}

fn failure(error: &BatchProviderReadError<ScriptedStatus>) -> Box<HelperError> {
    let (class, status): (&str, Option<u16>) = match error {
        BatchProviderReadError::Unsupported { .. }
        | BatchProviderReadError::InvalidVertexCredentials => ("BadRequestError", Some(400)),
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
fn echoed_batch() -> Result<BatchObject, Box<HelperError>> {
    let batch = concat!(
        r#"{"id":"batch_1","completion_window":"24h","created_at":0,"endpoint":"#,
        r#""/v1/chat/completions","input_file_id":"file_1","object":"batch","status":"completed"}"#,
    );
    Ok(serde_json::from_str(batch)?)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let raw: Raw = serde_json::from_value(json!({
            "litellm_settings": input.litellm_settings,
        }))?;
        let settings = raw.convert(&input.environment).or_config_error()?;
        let service = BatchProviderRead {
            dispatch: Scripted {
                script: input.script.clone(),
                batch: echoed_batch()?,
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
