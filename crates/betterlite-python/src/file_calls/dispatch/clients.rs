use std::time::Duration;

use betterlite_azure::azure_files::client::ClientSettings;
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_read::target::{AzureRead, CompatibleRead};
use betterlite_core::api::file_provider_write::ProviderCall;
use betterlite_core::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::{ClientArgs, FilesClient, get_openai_client, select_client};
use secrecy::{ExposeSecret as _, SecretString};

pub(crate) const fn settings(
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

// the two call shapes the files client is built from, the write inputs and the read inputs
#[derive(Clone, Copy)]
pub(super) enum ClientSource<'source> {
    Provider(&'source ProviderCall),
    Compatible(&'source CompatibleRead),
}

// get_openai_client then the files wrapper's own client check, as OpenAIFilesAPI makes them
pub(super) fn openai_client(source: ClientSource<'_>) -> Result<FilesClient, FilesError> {
    let args = match source {
        ClientSource::Provider(call) => ClientArgs {
            api_key: call.api_key.clone(),
            api_base: call.api_base.clone(),
            timeout: deadline(call.timeout),
            max_retries: call.max_retries.and_then(|count| u32::try_from(count).ok()),
            organization: call.organization.clone(),
        },
        ClientSource::Compatible(read) => ClientArgs {
            api_key: read.api_key.clone(),
            api_base: Some(read.api_base.clone()),
            timeout: deadline(read.mode.timeout),
            max_retries: read.max_retries.and_then(|count| u32::try_from(count).ok()),
            organization: read.organization.clone(),
        },
    };
    let asynchronous = match source {
        ClientSource::Provider(call) => is_asynchronous(call.mode),
        ClientSource::Compatible(read) => read.mode.is_async,
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

// Match the other SDK bridges: read, connect, write, pool, then the SDK default.
fn phase_seconds(phases: StructuredTimeout) -> f64 {
    phases
        .read
        .or(phases.connect)
        .or(phases.write)
        .or(phases.pool)
        .unwrap_or(600.0_f64)
}

pub(crate) fn deadline(timeout: FineTuningTimeout) -> Option<Duration> {
    match timeout {
        FineTuningTimeout::Seconds(seconds) => Duration::try_from_secs_f64(seconds).ok(),
        FineTuningTimeout::Structured(phases) => {
            Duration::try_from_secs_f64(phase_seconds(phases)).ok()
        }
    }
}

// base_llm_http_handler.create_file hands the azure handler the timeout it resolved from timeout or request_timeout
pub(crate) fn litellm_params(
    credentials: Option<&TrustedCredentials>,
    max_retries: Option<i64>,
    timeout: FineTuningTimeout,
) -> LiteLlmParams {
    LiteLlmParams {
        azure_ad_token: credentials
            .and_then(|trusted| trusted.values.get("azure_ad_token"))
            .cloned(),
        auth: auth_params(credentials),
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
        FineTuningTimeout::Structured(phases) => phase_seconds(phases),
    }
}

fn auth_params(credentials: Option<&TrustedCredentials>) -> AuthParams {
    let Some(trusted) = credentials else {
        return AuthParams::default();
    };
    AuthParams {
        tenant_id: trusted
            .values
            .get("tenant_id")
            .map(|secret| secret.expose_secret().to_owned()),
        client_id: trusted
            .values
            .get("client_id")
            .map(|secret| secret.expose_secret().to_owned()),
        client_secret: trusted.values.get("client_secret").cloned(),
        azure_username: trusted
            .values
            .get("azure_username")
            .map(|secret| secret.expose_secret().to_owned()),
        azure_password: trusted
            .values
            .get("azure_password")
            .map(|secret| SecretString::from(secret.expose_secret().to_owned())),
        azure_scope: trusted
            .values
            .get("azure_scope")
            .map(|secret| secret.expose_secret().to_owned()),
    }
}
