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

use super::call::AzureAssistantCall;
use super::request::{
    AssistantOperation, AssistantReadDeleteInput, AssistantTimeout, SuppliedClient,
    SuppliedClientKind,
};

// optional_params.timeout or kwargs.request_timeout or 600, where a zero or empty timeout is falsy
pub(super) fn resolve_timeout(
    input: &AssistantReadDeleteInput,
) -> Result<FineTuningTimeout, TimeoutError> {
    let timeout = input.params.timeout.as_ref();
    if let Some(phases) = timeout.and_then(AssistantTimeout::phases) {
        return resolve_fine_tuning_timeout(None, &input.provider, Some(phases));
    }
    match timeout.and_then(AssistantTimeout::value) {
        Some(value) => resolve_fine_tuning_timeout(Some(&value), &input.provider, None),
        None => Ok(FineTuningTimeout::Seconds(request_timeout(
            None,
            input.params.request_timeout,
        )?)),
    }
}

// The OpenAI SDK applies one read timeout, so a structured timeout contributes its read phase.
pub(super) fn timeout_seconds(timeout: FineTuningTimeout) -> f64 {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => seconds,
        FineTuningTimeout::Structured(timeout) => timeout.read.unwrap_or(600.0),
    }
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
        // the SDK compares its attempt count against max_retries, so a negative value behaves as zero
        max_retries: input.params.max_retries.map(|retries| {
            u32::try_from(retries).unwrap_or(if retries < 0 { 0 } else { u32::MAX })
        }),
        organization: settings.openai_organization(options).map(str::to_owned),
    }
}

// extra_body's azure_ad_token wins whenever extra_body exists; only a null extra_body reads the env
fn azure_ad_token(
    extra_body: &Option<Option<IndexMap<String, Json>>>,
    settings: &ResourceSdkSettings,
) -> (Option<SecretString>, IndexMap<String, Json>) {
    let Some(body) = extra_body else {
        return (None, IndexMap::new());
    };
    let Some(body) = body else {
        return (settings.azure_ad_token.clone(), IndexMap::new());
    };
    let mut remaining = body.clone();
    let token = remaining
        .shift_remove("azure_ad_token")
        .and_then(|value| match value {
            Json::Text(text) => Some(SecretString::from(text)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        });
    (token, remaining)
}

pub(super) fn azure(
    input: &AssistantReadDeleteInput,
    settings: &ResourceSdkSettings,
    options: &ResourceSdkRequestOptions,
    timeout: FineTuningTimeout,
) -> AzureAssistantCall {
    let (azure_ad_token, extra_body) = azure_ad_token(&input.extra_body, settings);
    AzureAssistantCall {
        api_base: settings.azure_base(options).map(str::to_owned),
        api_version: settings.azure_version(options).map(str::to_owned),
        api_key: settings.azure_api_key(options).cloned(),
        azure_ad_token,
        // initialize_azure_sdk_client accepts the facade timeout and never forwards it
        timeout: timeout_seconds(timeout),
        max_retries: input.params.max_retries,
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
            // litellm_params keeps the raw kwargs timeout, and B686 reads only a number from it
            timeout: input
                .params
                .timeout
                .as_ref()
                .and_then(|timeout| match timeout {
                    AssistantTimeout::Number(value) => Some(*value),
                    AssistantTimeout::Text(text) => text.trim().parse().ok(),
                    AssistantTimeout::Structured(_) => None,
                }),
        },
        extra_body,
    }
}

// an Azure delete drops a client of the OpenAI class, which AzureOpenAI subclasses; an async class passes
pub(super) fn supplied_client(input: &AssistantReadDeleteInput) -> Option<SuppliedClient> {
    let client = input.client.as_ref()?;
    let azure_delete = input.operation == AssistantOperation::Delete && input.provider == "azure";
    let openai_class = matches!(
        client.kind,
        SuppliedClientKind::OpenAi | SuppliedClientKind::AzureOpenAi
    );
    if azure_delete && openai_class {
        return None;
    }
    Some(client.clone())
}
