use std::error::Error;
use std::str::FromStr as _;

use betterlite_models::identity::provider_id::ProviderId;
use chrono::{DateTime, FixedOffset};

use crate::api::fine_tuning_read::ReadTimeout;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::error::{FileReadError, ReadFunction};
use super::target::{AzureRead, CompatibleRead, NewLogger, ReadMode};
use super::{Kwarg, ReadKwargs};

// GenericLiteLLMParams plus the call's mode, read once before any branch is chosen
pub(super) struct Plan<'call> {
    pub(super) function: ReadFunction,
    pub(super) kwargs: &'call ReadKwargs,
    pub(super) mode: ReadMode,
    pub(super) settings: ResourceSdkSettings,
    pub(super) options: ResourceSdkRequestOptions,
}

impl Plan<'_> {
    pub(super) fn compatible(&self) -> CompatibleRead {
        CompatibleRead {
            api_base: self.settings.openai_base(&self.options),
            api_key: self.settings.openai_api_key(&self.options).cloned(),
            organization: self
                .settings
                .openai_organization(&self.options)
                .map(str::to_owned),
            max_retries: self.kwargs.max_retries,
            mode: self.mode,
        }
    }

    pub(super) fn azure(&self) -> AzureRead {
        AzureRead {
            api_base: self.settings.azure_base(&self.options).map(str::to_owned),
            api_key: self.settings.azure_api_key(&self.options).cloned(),
            api_version: self
                .settings
                .azure_version(&self.options)
                .map(str::to_owned),
            max_retries: self.kwargs.max_retries,
            mode: self.mode,
        }
    }

    pub(super) fn new_logger(
        &self,
        start_time: DateTime<FixedOffset>,
        call_id: String,
    ) -> NewLogger {
        let call_type = match (self.function, self.mode.is_async) {
            (ReadFunction::FileRetrieve, false) => "file_retrieve",
            (ReadFunction::FileRetrieve, true) => "afile_retrieve",
            (ReadFunction::FileList, false) => "file_list",
            (ReadFunction::FileList, true) => "afile_list",
        };
        let litellm_call_id = match &self.kwargs.litellm_call_id {
            Kwarg::Absent => Some(call_id),
            Kwarg::Null => None,
            Kwarg::Text(given) => Some(given.clone()),
        };
        let function_id = match (&self.kwargs.id, self.function) {
            (Kwarg::Text(given), _) => given.clone(),
            // list writes str(kwargs.get("id", "")), so a None the caller passed becomes the text None
            (Kwarg::Null, ReadFunction::FileList) => "None".to_owned(),
            (Kwarg::Null, ReadFunction::FileRetrieve) | (Kwarg::Absent, _) => String::new(),
        };
        NewLogger {
            call_type,
            start_time,
            litellm_call_id,
            function_id,
        }
    }
}

// LlmProviders(custom_llm_provider): a name outside the enum is a ValueError before any branch answers
pub(super) fn known_provider<DispatchError>(
    name: &str,
) -> Result<ProviderId, FileReadError<DispatchError>>
where
    DispatchError: Error + 'static,
{
    ProviderId::from_str(name).map_err(|source| FileReadError::UnknownProvider {
        name: name.to_owned(),
        source,
    })
}

// timeout or request_timeout or 600, where a zero or an empty value falls through; an httpx.Timeout is never falsy
pub(super) fn read_timeout(
    kwargs: &ReadKwargs,
    provider: &str,
) -> Result<FineTuningTimeout, TimeoutError> {
    let plain = match &kwargs.timeout {
        Some(ReadTimeout::Structured(phases)) => {
            let pieces = StructuredTimeout::try_from(phases)?;
            return resolve_fine_tuning_timeout(None, provider, Some(pieces));
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
            return resolve_fine_tuning_timeout(Some(&value), provider, None);
        }
    }
    resolve_fine_tuning_timeout(None, provider, None)
}
