use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_openai::fine_tuning::job::decode_fine_tune_job;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::page::{self, FineTuningPage};
use super::{
    AzureReadCall, FineTuningReadDispatch, FineTuningReadError, FineTuningReadInput,
    FineTuningReadOutcome, FineTuningReadParams, OpenAiReadCall, ReadOperation, ReadProvider,
    ReadTarget, ReadTimeout, ReadTimeoutValue, ReadValues,
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
            FineTuningReadInput::List(call) => {
                self.read(
                    call.provider.as_ref(),
                    &call.params,
                    ReadOperation::List {
                        after: call.after.as_deref(),
                        limit: call.limit,
                    },
                    false,
                )
                .await
            }
            FineTuningReadInput::AList(call) => {
                self.read(
                    call.provider.as_ref(),
                    &call.params,
                    ReadOperation::List {
                        after: call.after.as_deref(),
                        limit: call.limit,
                    },
                    true,
                )
                .await
            }
            FineTuningReadInput::Retrieve(call) => {
                self.read(
                    call.provider.as_ref(),
                    &call.params,
                    ReadOperation::Retrieve {
                        job_id: &call.fine_tuning_job_id,
                    },
                    false,
                )
                .await
            }
            FineTuningReadInput::ARetrieve(call) => {
                self.read(
                    call.provider.as_ref(),
                    &call.params,
                    ReadOperation::Retrieve {
                        job_id: &call.fine_tuning_job_id,
                    },
                    true,
                )
                .await
            }
        }
    }

    // all four calls share the provider, timeout and dispatch chains; only the decode differs
    async fn read(
        &self,
        provider: Option<&ReadProvider>,
        params: &FineTuningReadParams,
        operation: ReadOperation<'_>,
        is_async: bool,
    ) -> Result<FineTuningReadOutcome, FineTuningReadError<Dispatch::Error>> {
        // a missing provider stays absent into the unsupported branch, where Python writes None
        let provider = match provider {
            Some(provider) => provider.0.as_str(),
            None => "None",
        };
        let timeout = resolve_timeout(params, provider)?;
        let settings = ResourceSdkSettings::from(&self.settings);
        let options = ResourceSdkRequestOptions::from(params);
        let values = ReadValues {
            is_async,
            operation,
            timeout: &timeout,
            max_retries: params.max_retries,
        };
        let Some(target) = read_target(provider, values, &settings, &options) else {
            let provider = provider.to_owned();
            return Err(match operation {
                ReadOperation::List { .. } => FineTuningReadError::UnsupportedList { provider },
                ReadOperation::Retrieve { .. } => {
                    FineTuningReadError::UnsupportedRetrieve { provider }
                }
            });
        };
        let is_azure = matches!(target, ReadTarget::Azure(_));
        let bytes = self
            .dispatch
            .read(target)
            .await
            .map_err(FineTuningReadError::Dispatch)?;
        match operation {
            ReadOperation::List { .. } => {
                let page: FineTuningPage = page::decode_page(&bytes)
                    .map_err(|source| FineTuningReadError::Page { source })?;
                Ok(FineTuningReadOutcome::Page(page))
            }
            ReadOperation::Retrieve { .. } => {
                let job = decode_fine_tune_job(&bytes, is_azure)?;
                Ok(FineTuningReadOutcome::Job(Box::new(job)))
            }
        }
    }
}

// both read calls build the same two provider chains, and only their unsupported message differs
fn read_target<'call>(
    provider: &str,
    values: ReadValues<'call>,
    settings: &'call ResourceSdkSettings,
    options: &'call ResourceSdkRequestOptions,
) -> Option<ReadTarget<'call>> {
    match provider {
        "openai" => Some(ReadTarget::OpenAi(OpenAiReadCall {
            values,
            api_base: settings.openai_base(options),
            api_key: settings.openai_api_key(options),
            api_version: settings.openai_api_version(options),
            organization: settings.openai_organization(options),
        })),
        "azure" => Some(ReadTarget::Azure(AzureReadCall {
            values,
            api_base: settings.azure_base(options),
            api_key: settings.azure_api_key(options),
            api_version: settings.azure_version(options),
            organization: settings.azure_organization(options),
        })),
        _ => None,
    }
}

// optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
fn resolve_timeout(
    params: &FineTuningReadParams,
    provider: &str,
) -> Result<FineTuningTimeout, TimeoutError> {
    match params.timeout.as_ref() {
        Some(ReadTimeout::Structured(structured)) => {
            resolve_fine_tuning_timeout(None, provider, Some(StructuredTimeout::from(*structured)))
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
