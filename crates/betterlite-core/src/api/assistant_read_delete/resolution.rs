use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::call::{AzureAssistantCall, AzureCallRequest};
use super::request::{AssistantOperation, AssistantReadDeleteInput, AssistantTimeout, ExtraBody};

// optional_params.timeout or kwargs.request_timeout or 600, where a zero or empty timeout is falsy
pub(super) fn resolve_timeout(
    input: &AssistantReadDeleteInput,
) -> Result<FineTuningTimeout, TimeoutError> {
    let provider = input.provider.as_str();
    match configured(input.params.timeout.as_ref()) {
        Configured::Structured(timeout) => {
            resolve_fine_tuning_timeout(None, provider, Some(timeout))
        }
        Configured::Plain(value) => resolve_fine_tuning_timeout(Some(value), provider, None),
        Configured::Unset | Configured::Zero => Ok(FineTuningTimeout::Seconds(request_timeout(
            None,
            input.params.request_timeout,
        )?)),
    }
}

enum Configured<'value> {
    Unset,
    Zero,
    Plain(&'value TimeoutValue),
    Structured(StructuredTimeout),
}

fn configured(timeout: Option<&AssistantTimeout>) -> Configured<'_> {
    match timeout {
        None => Configured::Unset,
        Some(AssistantTimeout::Structured(timeout)) => Configured::Structured(*timeout),
        Some(AssistantTimeout::Plain(TimeoutValue::Number(value))) if *value == 0.0 => {
            Configured::Zero
        }
        Some(AssistantTimeout::Plain(TimeoutValue::String(text))) if text.is_empty() => {
            Configured::Zero
        }
        Some(AssistantTimeout::Plain(value)) => Configured::Plain(value),
    }
}

// The OpenAI SDK applies one read timeout, so a structured timeout contributes its read phase.
pub(super) fn timeout_seconds(timeout: FineTuningTimeout) -> f64 {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => seconds,
        FineTuningTimeout::Structured(timeout) => timeout.read.unwrap_or(600.0),
    }
}

// the SDK compares its attempt count against max_retries, so a negative value behaves as zero
fn max_retries(value: Option<i64>) -> Option<u32> {
    value.map(|retries| {
        u32::try_from(retries).unwrap_or_else(|_| if retries < 0 { 0 } else { u32::MAX })
    })
}

pub(super) fn openai(
    input: &AssistantReadDeleteInput,
    settings: &ResourceSdkSettings,
    options: &ResourceSdkRequestOptions,
    timeout: FineTuningTimeout,
) -> ClientOptions {
    ClientOptions {
        api_key: settings.openai_api_key(options).cloned(),
        // LiteLLM passes the resolved default as base_url, so this branch always names a base
        api_base: Some(settings.openai_base(options)),
        timeout: std::time::Duration::try_from_secs_f64(timeout_seconds(timeout)).ok(),
        max_retries: max_retries(input.params.max_retries),
        organization: settings.openai_organization(options).map(str::to_owned),
    }
}

// litellm_params keeps the raw kwargs timeout, and B686 reads only a number from it
fn params_timeout(timeout: Option<&AssistantTimeout>) -> Option<f64> {
    match timeout {
        None => None,
        Some(AssistantTimeout::Plain(TimeoutValue::Number(value))) => Some(*value),
        Some(AssistantTimeout::Plain(TimeoutValue::String(text))) => text.trim().parse().ok(),
        Some(AssistantTimeout::Structured(_)) => None,
    }
}

// extra_body's azure_ad_token wins whenever extra_body exists; only a null extra_body reads the env
fn azure_ad_token(
    extra_body: &ExtraBody,
    settings: &ResourceSdkSettings,
) -> (Option<SecretString>, IndexMap<String, Json>) {
    match extra_body {
        ExtraBody::Missing => (None, IndexMap::new()),
        ExtraBody::Null => (settings.azure_ad_token.clone(), IndexMap::new()),
        ExtraBody::Body(body) => {
            let mut remaining = body.clone();
            let token = remaining.shift_remove("azure_ad_token").and_then(json_text);
            (token, remaining)
        }
    }
}

fn json_text(value: Json) -> Option<SecretString> {
    match value {
        Json::Text(text) => Some(SecretString::from(text)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn azure(
    input: &AssistantReadDeleteInput,
    settings: &ResourceSdkSettings,
    options: &ResourceSdkRequestOptions,
    timeout: FineTuningTimeout,
) -> AzureAssistantCall {
    let (azure_ad_token, extra_body) = azure_ad_token(&input.extra_body, settings);
    AzureAssistantCall {
        request: AzureCallRequest {
            api_base: settings.azure_base(options).map(str::to_owned),
            api_version: settings.azure_version(options).map(str::to_owned),
            api_key: settings.azure_api_key(options).cloned(),
            azure_ad_token,
            // initialize_azure_sdk_client accepts the facade timeout and never forwards it
            timeout: timeout_seconds(timeout),
            max_retries: input.params.max_retries,
        },
        params: LiteLlmParams {
            // litellm_params carries the top-level azure_ad_token kwarg, not extra_body's or the env's
            azure_ad_token: input.params.azure_ad_token.clone(),
            auth: AuthParams {
                tenant_id: input.params.tenant_id.clone(),
                client_id: input.params.client_id.clone(),
                client_secret: input.params.client_secret.clone(),
                azure_username: input.params.azure_username.clone(),
                azure_password: input.params.azure_password.clone(),
                azure_scope: input.params.azure_scope.clone(),
            },
            max_retries: input.params.max_retries,
            timeout: params_timeout(input.params.timeout.as_ref()),
        },
        extra_body,
    }
}

// an Azure delete drops a client of the OpenAI class, which AzureOpenAI subclasses; an async class passes
pub(super) fn supplied_client(
    input: &AssistantReadDeleteInput,
) -> Option<super::request::SuppliedClient> {
    let client = input.client.as_ref()?;
    let azure_delete = input.operation == AssistantOperation::Delete && input.provider == "azure";
    if azure_delete && client.kind.is_openai_instance() {
        return None;
    }
    Some(client.clone())
}
