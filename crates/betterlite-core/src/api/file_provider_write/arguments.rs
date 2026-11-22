use std::error::Error;
use std::str::FromStr as _;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::files::records::{CreatePurpose, FileCreate};

use crate::api::file_http_upload::service::Mode;
use crate::api::resource_sdk_settings::ResourceSdkRequestOptions;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};

use super::{DeleteParams, FileWriteError, SuppliedClient, UploadInput, WriteParams, WriteTimeout};
// an absent provider takes the function's default: None for create_file, openai for acreate_file
pub(super) fn upload_provider<Logger, Client>(
    mode: Mode,
    input: &UploadInput<Logger, Client>,
) -> Option<String> {
    input
        .custom_llm_provider
        .clone()
        .unwrap_or_else(|| match mode {
            Mode::Sync => None,
            Mode::Asynchronous => Some("openai".to_owned()),
        })
}
// CreateFileRequest carries expires_after only when the caller set it
pub(super) fn create_request<Logger, Client>(input: &UploadInput<Logger, Client>) -> FileCreate {
    FileCreate {
        file: input.file.clone(),
        purpose: input.purpose,
        expires_after: input.expires_after,
        extra_headers: input.extra_headers.clone().unwrap_or_default(),
        extra_body: input.extra_body.clone().unwrap_or_default(),
        timeout: None,
    }
}
// only the literal True arms the guard, and then only a vertex_ai batch passes it
pub(super) fn passthrough_guard<ProviderError: Error + 'static>(
    passthrough: Option<bool>,
    provider: Option<&str>,
    purpose: CreatePurpose,
) -> Result<(), FileWriteError<ProviderError>> {
    if passthrough == Some(true)
        && (provider != Some("vertex_ai") || purpose != CreatePurpose::Batch)
    {
        return Err(FileWriteError::Passthrough {
            provider: provider.map(str::to_owned),
            purpose: purpose.wire_name(),
        });
    }
    Ok(())
}
pub(super) fn request_options<Logger, Client>(
    params: &WriteParams<Logger, Client>,
) -> ResourceSdkRequestOptions {
    ResourceSdkRequestOptions {
        api_base: params.api_base.clone(),
        api_version: params.api_version.clone(),
        api_key: params.api_key.clone(),
        organization: params.organization.clone(),
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
    }
}
// the generic handler takes a supplied client only when it is an httpx handler
pub(super) fn handler<Logger, Client>(
    params: &WriteParams<Logger, Client>,
) -> Option<&SuppliedClient<Client>> {
    params
        .client
        .as_ref()
        .filter(|client| matches!(client, SuppliedClient::Handler { .. }))
}
pub(super) fn delete_params<Logger, Client>(
    params: &WriteParams<Logger, Client>,
) -> DeleteParams<'_> {
    DeleteParams {
        api_key: params.api_key.as_ref(),
        api_base: params.api_base.as_deref(),
        credentials: params.credentials.as_ref(),
    }
}
// LlmProviders(custom_llm_provider): an unknown or missing name is a ValueError naming its repr
pub(super) fn provider_id<ProviderError: Error + 'static>(
    provider: Option<&str>,
) -> Result<ProviderId, FileWriteError<ProviderError>> {
    let unknown = || FileWriteError::UnknownProvider {
        provider: provider.map_or_else(|| "None".to_owned(), |name| format!("'{name}'")),
    };
    let name = provider.ok_or_else(unknown)?;
    ProviderId::from_str(name).map_err(|_unparsed| unknown())
}
// optional_params.timeout or request_timeout or 600, where zero and empty fall through; an httpx.Timeout keeps its phases only for openai, azure and bedrock
pub(super) fn write_timeout<Logger, Client, ProviderError: Error + 'static>(
    params: &WriteParams<Logger, Client>,
    provider: &str,
) -> Result<FineTuningTimeout, FileWriteError<ProviderError>> {
    let selected = match params.timeout.as_ref() {
        Some(WriteTimeout::Structured(structured)) => {
            return resolve_fine_tuning_timeout(None, provider, Some(*structured))
                .map_err(FileWriteError::Timeout);
        }
        Some(WriteTimeout::Value(TimeoutValue::Number(seconds))) if *seconds != 0.0_f64 => {
            Some(TimeoutValue::Number(*seconds))
        }
        Some(WriteTimeout::Value(TimeoutValue::String(text))) if !text.is_empty() => {
            Some(TimeoutValue::String(text.clone()))
        }
        None | Some(WriteTimeout::Value(_)) => params.request_timeout.clone(),
    };
    resolve_fine_tuning_timeout(selected.as_ref(), provider, None).map_err(|error| match selected {
        Some(TimeoutValue::String(text)) if matches!(error, TimeoutError::InvalidNumber { .. }) => {
            FileWriteError::TimeoutText {
                text,
                source: error,
            }
        }
        None | Some(TimeoutValue::Number(_) | TimeoutValue::String(_)) => {
            FileWriteError::Timeout(error)
        }
    })
}
