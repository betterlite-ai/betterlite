use std::error::Error;

use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_openai::fine_tuning::job::decode_fine_tune_job;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::page;
use super::{
    AzureReadCall, FineTuningReadDispatch, FineTuningReadError, FineTuningReadInput,
    FineTuningReadOutcome, FineTuningReadParams, OpenAiReadCall, ReadOperation, ReadProvider,
    ReadTarget, ReadTimeout, ReadValues,
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
                    call.provider.0.as_ref(),
                    &call.params,
                    call.operation(),
                    false,
                )
                .await
            }
            FineTuningReadInput::AList(call) => {
                self.read(
                    call.provider.0.as_ref(),
                    &call.params,
                    call.operation(),
                    true,
                )
                .await
            }
            FineTuningReadInput::Retrieve(call) => {
                self.read(
                    call.provider.0.as_ref(),
                    &call.params,
                    call.operation(),
                    false,
                )
                .await
            }
            FineTuningReadInput::ARetrieve(call) => {
                self.read(
                    call.provider.0.as_ref(),
                    &call.params,
                    call.operation(),
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
        let provider_name = provider.map_or("None", |named| named.0.as_str());
        let timeout = resolve_timeout(params, provider_name)?;
        let settings = ResourceSdkSettings::from(&self.settings);
        let options = ResourceSdkRequestOptions::from(params);
        let values = ReadValues {
            is_async,
            operation,
            timeout: &timeout,
            max_retries: params.max_retries,
        };
        let Some(target) = read_target(provider_name, values, &settings, &options) else {
            return Err(Self::unsupported(provider_name, operation));
        };
        let is_azure = matches!(target, ReadTarget::Azure(_));
        let bytes = self
            .dispatch
            .read(target)
            .await
            .map_err(FineTuningReadError::Dispatch)?;
        decode(operation, &bytes, is_azure)
    }

    // list keeps LiteLLM's create_batch wording, and retrieve names its own function
    fn unsupported(
        provider: &str,
        operation: ReadOperation<'_>,
    ) -> FineTuningReadError<Dispatch::Error> {
        match operation {
            ReadOperation::List { .. } => FineTuningReadError::UnsupportedList {
                provider: provider.to_owned(),
            },
            ReadOperation::Retrieve { .. } => FineTuningReadError::UnsupportedRetrieve {
                provider: provider.to_owned(),
            },
        }
    }
}

// list decodes a page, and retrieve decodes a job the azure handler normalizes first
fn decode<DispatchError>(
    operation: ReadOperation<'_>,
    bytes: &[u8],
    is_azure: bool,
) -> Result<FineTuningReadOutcome, FineTuningReadError<DispatchError>>
where
    DispatchError: Error + 'static,
{
    match operation {
        ReadOperation::List { .. } => Ok(FineTuningReadOutcome::Page(
            page::decode_page(bytes).map_err(|source| FineTuningReadError::Page { source })?,
        )),
        ReadOperation::Retrieve { .. } => Ok(FineTuningReadOutcome::Job(Box::new(
            decode_fine_tune_job(bytes, is_azure)?,
        ))),
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
    let configured = match params.timeout.as_ref() {
        Some(ReadTimeout::Structured(structured)) => {
            return resolve_fine_tuning_timeout(
                None,
                provider,
                Some(StructuredTimeout::try_from(structured)?),
            );
        }
        Some(ReadTimeout::Plain(value)) => Some(TimeoutValue::try_from(value)?),
        None => None,
    };
    let selected = match configured {
        Some(TimeoutValue::Number(seconds)) if seconds != 0.0_f64 => {
            Some(TimeoutValue::Number(seconds))
        }
        Some(TimeoutValue::String(text)) if !text.is_empty() => Some(TimeoutValue::String(text)),
        _ => params
            .request_timeout
            .as_ref()
            .map(TimeoutValue::try_from)
            .transpose()?,
    };
    resolve_fine_tuning_timeout(selected.as_ref(), provider, None)
}
