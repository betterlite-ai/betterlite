use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_openai::fine_tuning::job::decode_fine_tune_job;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::page::{self, FineTuningPage};
use super::{
    AzureReadCall, FineTuningListCall, FineTuningReadDispatch, FineTuningReadError,
    FineTuningReadInput, FineTuningReadOutcome, FineTuningReadParams, FineTuningRetrieveCall,
    OpenAiReadCall, ReadOperation, ReadProvider, ReadTarget, ReadTimeout, ReadTimeoutValue,
    ReadValues,
};

pub struct FineTuningRead<Dispatch> {
    pub dispatch: Dispatch,
    pub settings: ConfiguredSettings,
}

impl<Dispatch> FineTuningRead<Dispatch>
where
    Dispatch: FineTuningReadDispatch,
{
    pub async fn run(
        &self,
        input: &FineTuningReadInput,
    ) -> Result<FineTuningReadOutcome, FineTuningReadError<Dispatch::Error>> {
        match input {
            FineTuningReadInput::List(call) => self.list(call, false).await,
            FineTuningReadInput::AList(call) => self.list(call, true).await,
            FineTuningReadInput::Retrieve(call) => self.retrieve(call, false).await,
            FineTuningReadInput::ARetrieve(call) => self.retrieve(call, true).await,
        }
    }

    async fn list(
        &self,
        call: &FineTuningListCall,
        is_async: bool,
    ) -> Result<FineTuningReadOutcome, FineTuningReadError<Dispatch::Error>> {
        let provider = provider_name(call.provider.as_ref());
        let timeout = resolve_timeout(&call.params, provider)?;
        let settings = ResourceSdkSettings::from(&self.settings);
        let options = ResourceSdkRequestOptions::from(&call.params);
        let values = ReadValues {
            is_async,
            operation: ReadOperation::List {
                after: call.after.as_deref(),
                limit: call.limit,
            },
            timeout: &timeout,
            max_retries: call.params.max_retries,
        };
        let target = match provider {
            "openai" => ReadTarget::OpenAi(OpenAiReadCall {
                values,
                api_base: settings.openai_base(&options),
                api_key: settings.openai_api_key(&options),
                api_version: settings.openai_api_version(&options),
                organization: settings.openai_organization(&options),
            }),
            "azure" => ReadTarget::Azure(AzureReadCall {
                values,
                api_base: settings.azure_base(&options),
                api_key: settings.azure_api_key(&options),
                api_version: settings.azure_version(&options),
                organization: settings.azure_organization(&options),
            }),
            _ => {
                return Err(FineTuningReadError::UnsupportedList {
                    provider: provider.to_owned(),
                });
            }
        };
        let bytes = self
            .dispatch
            .read(target)
            .await
            .map_err(FineTuningReadError::Dispatch)?;
        let page: FineTuningPage =
            page::decode_page(&bytes).map_err(|source| FineTuningReadError::Page { source })?;
        Ok(FineTuningReadOutcome::Page(page))
    }

    // only retrieve decodes a single job, so only its azure answer is normalized
    async fn retrieve(
        &self,
        call: &FineTuningRetrieveCall,
        is_async: bool,
    ) -> Result<FineTuningReadOutcome, FineTuningReadError<Dispatch::Error>> {
        let provider = provider_name(call.provider.as_ref());
        let timeout = resolve_timeout(&call.params, provider)?;
        let settings = ResourceSdkSettings::from(&self.settings);
        let options = ResourceSdkRequestOptions::from(&call.params);
        let values = ReadValues {
            is_async,
            operation: ReadOperation::Retrieve {
                job_id: &call.fine_tuning_job_id,
            },
            timeout: &timeout,
            max_retries: call.params.max_retries,
        };
        let target = match provider {
            "openai" => ReadTarget::OpenAi(OpenAiReadCall {
                values,
                api_base: settings.openai_base(&options),
                api_key: settings.openai_api_key(&options),
                api_version: settings.openai_api_version(&options),
                organization: settings.openai_organization(&options),
            }),
            "azure" => ReadTarget::Azure(AzureReadCall {
                values,
                api_base: settings.azure_base(&options),
                api_key: settings.azure_api_key(&options),
                api_version: settings.azure_version(&options),
                organization: settings.azure_organization(&options),
            }),
            _ => {
                return Err(FineTuningReadError::UnsupportedRetrieve {
                    provider: provider.to_owned(),
                });
            }
        };
        let is_azure = matches!(target, ReadTarget::Azure(_));
        let bytes = self
            .dispatch
            .read(target)
            .await
            .map_err(FineTuningReadError::Dispatch)?;
        let job = decode_fine_tune_job(&bytes, is_azure)?;
        Ok(FineTuningReadOutcome::Job(Box::new(job)))
    }
}

// a missing provider stays absent into the unsupported branch, where Python writes None
fn provider_name(provider: Option<&ReadProvider>) -> &str {
    provider.map_or("None", |provider| provider.0.as_str())
}

// optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
fn resolve_timeout(
    params: &FineTuningReadParams,
    provider: &str,
) -> Result<FineTuningTimeout, TimeoutError> {
    match params.timeout.as_ref() {
        Some(ReadTimeout::Structured(structured)) => {
            resolve_fine_tuning_timeout(None, provider, Some((*structured).into()))
        }
        Some(ReadTimeout::Plain(value)) if configured(value) => {
            let raw = TimeoutValue::from(value);
            resolve_fine_tuning_timeout(Some(&raw), provider, None)
        }
        None | Some(ReadTimeout::Plain(_)) => match params.request_timeout.as_ref() {
            Some(value) => {
                let raw = TimeoutValue::from(value);
                resolve_fine_tuning_timeout(Some(&raw), provider, None)
            }
            None => resolve_fine_tuning_timeout(None, provider, None),
        },
    }
}

// a value Python reads as falsy lets request_timeout take the chain
fn configured(value: &ReadTimeoutValue) -> bool {
    match value {
        ReadTimeoutValue::Number(seconds) => *seconds != 0.0_f64,
        ReadTimeoutValue::Text(text) => !text.is_empty(),
    }
}
