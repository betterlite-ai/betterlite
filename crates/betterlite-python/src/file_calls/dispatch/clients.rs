use std::time::Duration;

use betterlite_azure::azure_files::client::ClientSettings;
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_read::target::{AzureRead, CompatibleRead};
use betterlite_core::api::file_provider_write::{
    DeleteParams, ProviderCall, WriteParams, WriteTimeout,
};
use betterlite_core::api::resource_sdk_settings::timeout::{FineTuningTimeout, TimeoutValue};
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::{ClientArgs, FilesClient, get_openai_client, select_client};
use secrecy::{ExposeSecret as _, SecretString};

pub(crate) fn settings(
    api_key: Option<SecretString>,
    api_base: Option<String>,
    api_version: Option<String>,
) -> ClientSettings {
    ClientSettings {
        api_key,
        api_base,
        api_version,
    }
}

// get_openai_client then the files wrapper's own client check, as OpenAIFilesAPI makes them
pub(super) fn openai_client(call: &ProviderCall) -> Result<FilesClient, FilesError> {
    let asynchronous = is_asynchronous(call.mode);
    let args = ClientArgs {
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        timeout: deadline(call.timeout),
        max_retries: call.max_retries.and_then(|count| u32::try_from(count).ok()),
        organization: call.organization.clone(),
    };
    select_client(get_openai_client(args, None, asynchronous)?, asynchronous)
}

// the flag both client builders read, kept off every comparison so the sync and async modes are the only shapes
pub(crate) const fn is_asynchronous(mode: Mode) -> bool {
    match mode {
        Mode::Sync => false,
        Mode::Asynchronous => true,
    }
}

pub(super) fn compatible_client(read: &CompatibleRead) -> Result<FilesClient, FilesError> {
    let args = ClientArgs {
        api_key: read.api_key.clone(),
        api_base: Some(read.api_base.clone()),
        timeout: deadline(read.mode.timeout),
        max_retries: read.max_retries.and_then(|count| u32::try_from(count).ok()),
        organization: read.organization.clone(),
    };
    select_client(
        get_openai_client(args, None, read.mode.is_async)?,
        read.mode.is_async,
    )
}

// Match the other SDK bridges: read, connect, write, pool, then the SDK default.
pub(crate) fn deadline(timeout: FineTuningTimeout) -> Option<Duration> {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => Duration::try_from_secs_f64(seconds).ok(),
        FineTuningTimeout::Structured(phases) => {
            let seconds = phases
                .read
                .or(phases.connect)
                .or(phases.write)
                .or(phases.pool)
                .unwrap_or(600.0_f64);
            Duration::try_from_secs_f64(seconds).ok()
        }
    }
}

// base_llm_http_handler.create_file hands the azure handler the caller's kwargs, so timeout and max_retries reach the client
pub(crate) fn write_litellm_params(params: &WriteParams<(), ()>) -> LiteLlmParams {
    LiteLlmParams {
        azure_ad_token: credential(params.credentials.as_ref(), "azure_ad_token"),
        auth: auth_params(params.credentials.as_ref()),
        max_retries: params.max_retries,
        timeout: write_deadline(params.timeout.as_ref()),
    }
}

// azure's delete_file method gets litellm_params too, but its timeout is the one the call resolved
pub(crate) fn delete_litellm_params(
    params: &DeleteParams<'_>,
    timeout: FineTuningTimeout,
    max_retries: Option<i64>,
) -> LiteLlmParams {
    LiteLlmParams {
        azure_ad_token: credential(params.credentials, "azure_ad_token"),
        auth: auth_params(params.credentials),
        max_retries,
        timeout: Some(seconds_of(timeout)),
    }
}

// azure's retrieve_file and list_files take the call's timeout and max_retries and no credentials
pub(crate) fn read_litellm_params(read: &AzureRead) -> LiteLlmParams {
    LiteLlmParams {
        azure_ad_token: None,
        auth: AuthParams::default(),
        max_retries: read.max_retries,
        timeout: Some(seconds_of(read.mode.timeout)),
    }
}

fn seconds_of(timeout: FineTuningTimeout) -> f64 {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => seconds,
        FineTuningTimeout::Structured(phases) => phases
            .read
            .or(phases.connect)
            .or(phases.write)
            .or(phases.pool)
            .unwrap_or(600.0_f64),
    }
}

pub(crate) fn write_deadline(timeout: Option<&WriteTimeout>) -> Option<f64> {
    match timeout {
        None => None,
        Some(WriteTimeout::Value(TimeoutValue::Number(seconds))) => Some(*seconds),
        Some(WriteTimeout::Value(TimeoutValue::String(text))) => text.parse::<f64>().ok(),
        Some(WriteTimeout::Structured(phases)) => Some(
            phases
                .read
                .or(phases.connect)
                .or(phases.write)
                .or(phases.pool)
                .unwrap_or(600.0_f64),
        ),
    }
}

fn auth_params(credentials: Option<&TrustedCredentials>) -> AuthParams {
    AuthParams {
        tenant_id: credential_text(credentials, "tenant_id"),
        client_id: credential_text(credentials, "client_id"),
        client_secret: credential(credentials, "client_secret"),
        azure_username: credential_text(credentials, "azure_username"),
        azure_password: credential_text(credentials, "azure_password"),
        azure_scope: credential_text(credentials, "azure_scope"),
    }
}

fn credential(credentials: Option<&TrustedCredentials>, name: &str) -> Option<SecretString> {
    let values = credentials?.values.get(name)?;
    Some(values.clone())
}

fn credential_text(credentials: Option<&TrustedCredentials>, name: &str) -> Option<String> {
    let secret = credentials?.values.get(name)?;
    Some(secret.expose_secret().to_owned())
}
