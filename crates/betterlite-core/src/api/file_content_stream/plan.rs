use std::error::Error;
use std::str::FromStr as _;

use betterlite_auth::auth_model_lists::providers::is_declared_provider;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use chrono::{DateTime, FixedOffset};

use crate::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutValue, resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::error::FileContentError;
use super::target::{AzureContent, CompatibleContent, ContentMode, NewLogger, SuppliedSdk};
use super::{ContentCall, ContentEntry, ContentKwargs};

// GenericLiteLLMParams plus the call's mode, read once before any branch is chosen
pub(super) struct Plan<'call> {
    pub(super) kwargs: &'call ContentKwargs,
    pub(super) mode: ContentMode,
    pub(super) settings: ResourceSdkSettings,
    pub(super) options: ResourceSdkRequestOptions,
}

impl Plan<'_> {
    pub(super) fn compatible(&self, client: Option<SuppliedSdk>) -> CompatibleContent {
        CompatibleContent {
            api_base: self.settings.openai_base(&self.options),
            api_key: self.settings.openai_api_key(&self.options).cloned(),
            organization: self
                .settings
                .openai_organization(&self.options)
                .map(str::to_owned),
            max_retries: self.kwargs.max_retries,
            mode: self.mode,
            client,
        }
    }

    // get_azure_credentials' endpoint, key and version, each read from the call, then the settings, then the environment
    pub(super) fn azure(&self, client: Option<SuppliedSdk>) -> AzureContent {
        let settings = &self.settings;
        let options = &self.options;
        AzureContent {
            api_base: settings.azure_base(options).map(str::to_owned),
            api_key: settings.azure_api_key(options).cloned(),
            api_version: settings.azure_version(options).map(str::to_owned),
            max_retries: self.kwargs.max_retries,
            mode: self.mode,
            client,
        }
    }

    // _file_content_logging_obj: the call type names the entry, the id is str(truthy value or a UUID) and the function id str(value or "")
    pub(super) fn new_logger(
        &self,
        start_time: DateTime<FixedOffset>,
        generated_id: String,
    ) -> NewLogger {
        NewLogger {
            call_type: if self.mode.is_async {
                "afile_content"
            } else {
                "file_content"
            },
            start_time,
            litellm_call_id: self
                .kwargs
                .litellm_call_id
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or(generated_id),
            function_id: self.kwargs.id.clone().unwrap_or_default(),
        }
    }
}

// afile_content declares custom_llm_provider="openai"; file_content's undecorated default is None
pub(super) fn declared_provider(entry: ContentEntry, call: &ContentCall<'_>) -> Option<String> {
    call.custom_llm_provider.clone().or_else(|| match entry {
        ContentEntry::Sync => None,
        ContentEntry::Async => Some(String::from("openai")),
    })
}

// get_llm_provider(model, custom_llm_provider) may rename the provider; LiteLLM swallows its failure
pub(super) fn resolved_provider(
    entry: ContentEntry,
    call: &ContentCall<'_>,
    resolution: &ResolutionContext<'_>,
) -> Option<String> {
    let declared = declared_provider(entry, call);
    let Some(model) = call.model else {
        return declared;
    };
    let request = ResolutionRequest {
        model,
        custom_llm_provider: declared.as_deref(),
        api_base: None,
        api_key: None,
    };
    match resolve(&request, resolution) {
        Ok(found) => Some(found.provider.slug().to_owned()),
        Err(_error) => declared,
    }
}

// timeout, then request_timeout, then 600; structured phases only for openai, azure and bedrock
pub(super) fn content_timeout<DispatchError>(
    kwargs: &ContentKwargs,
    provider: Option<&str>,
) -> Result<FineTuningTimeout, FileContentError<DispatchError>>
where
    DispatchError: Error + 'static,
{
    let provider = provider.unwrap_or("None");
    let plain = match &kwargs.timeout {
        Some(ReadTimeout::Structured(phases)) => {
            let pieces = StructuredTimeout::try_from(phases)?;
            return resolve_fine_tuning_timeout(None, provider, Some(pieces))
                .map_err(FileContentError::Timeout);
        }
        Some(ReadTimeout::Plain(value)) => Some(value),
        None => None,
    };
    for candidate in [plain, kwargs.request_timeout.as_ref()]
        .into_iter()
        .flatten()
    {
        let value = TimeoutValue::try_from(candidate)?;
        let given = match &value {
            TimeoutValue::Number(seconds) => *seconds != 0.0_f64,
            TimeoutValue::String(text) => !text.is_empty(),
        };
        if given {
            return resolve_fine_tuning_timeout(Some(&value), provider, None)
                .map_err(FileContentError::Timeout);
        }
    }
    resolve_fine_tuning_timeout(None, provider, None).map_err(FileContentError::Timeout)
}

// LlmProviders(custom_llm_provider): an unknown or missing name is a ValueError naming its repr
pub(super) fn known_provider<DispatchError>(
    provider: Option<&str>,
) -> Result<ProviderId, FileContentError<DispatchError>>
where
    DispatchError: Error + 'static,
{
    let unknown = || FileContentError::UnknownProvider {
        provider: provider.map_or_else(|| String::from("None"), |name| format!("'{name}'")),
    };
    let Some(name) = provider else {
        return Err(unknown());
    };
    if !is_declared_provider(name) {
        return Err(unknown());
    }
    ProviderId::from_str(name).map_err(|_unparsed| unknown())
}
