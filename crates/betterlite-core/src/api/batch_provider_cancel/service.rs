use std::error::Error;
use std::sync::Arc;

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::BatchObject;
use indexmap::IndexMap;

use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary, JsonBoundary,
};
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::async_invoke;
use super::connection;
use super::{
    AzureCancelCall, BatchCancelDispatch, BatchProviderCancelError, BatchProviderCancelInput,
    BatchProviderCancelResult, BedrockCancelCall, CancelBatchCall, CancelCall, CancelTarget,
    CancelValues, OpenAiCancelCall, VertexCancelCall, XaiCancelCall,
};

pub struct BatchProviderCancel<Dispatch> {
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub dispatch: Dispatch,
    pub limits: ApiLimits,
    pub settings: Settings,
}

// the values every provider branch reads after the shared limit and timeout checks
struct Prepared {
    is_async: bool,
    request: BatchIdRequest,
    timeout: FineTuningTimeout,
    settings: ResourceSdkSettings,
    options: ResourceSdkRequestOptions,
}

impl<Dispatch> BatchProviderCancel<Dispatch>
where
    Dispatch: BatchCancelDispatch,
    Dispatch::Error: Error + Send + Sync + 'static,
{
    pub async fn run(
        &self,
        input: &BatchProviderCancelInput,
    ) -> Result<BatchProviderCancelResult, BatchProviderCancelError<Dispatch::Error>> {
        match input {
            BatchProviderCancelInput::Cancel(call) => self.cancel(call).await,
            BatchProviderCancelInput::AsyncInvoke(status) => Ok(async_invoke::normalize(
                status,
                (self.clock)(),
                &self.limits,
            )?),
        }
    }

    async fn cancel(
        &self,
        call: &CancelBatchCall,
    ) -> Result<BatchProviderCancelResult, BatchProviderCancelError<Dispatch::Error>> {
        check_cancel_limits(&self.limits, call)?;
        let prepared = Prepared {
            is_async: matches!(&call.call, CancelCall::Async { .. }),
            request: BatchIdRequest {
                batch_id: Some(call.batch_id.clone()),
                extra_headers: call.extra_headers.clone(),
                extra_body: call.extra_body.clone(),
                timeout: None,
            },
            timeout: connection::resolve_timeout(call)?,
            settings: ResourceSdkSettings::from(&self.settings),
            options: ResourceSdkRequestOptions::from(&call.params),
        };
        let batch = self.dispatch_branch(call, &prepared).await?;
        Ok(BatchProviderCancelResult {
            batch,
            model: effective_model(call),
        })
    }

    // cancel_batch branches on the provider's wire name; a name it doesn't declare answers 400
    async fn dispatch_branch(
        &self,
        call: &CancelBatchCall,
        prepared: &Prepared,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        let Some(target) = cancel_target(call, prepared) else {
            return Err(BatchProviderCancelError::Unsupported {
                provider: call.provider.0.clone(),
            });
        };
        self.dispatch
            .cancel(target)
            .await
            .map_err(BatchProviderCancelError::Dispatch)
    }
}

fn cancel_values<'call>(
    call: &'call CancelBatchCall,
    prepared: &'call Prepared,
) -> CancelValues<'call> {
    CancelValues {
        is_async: prepared.is_async,
        batch_id: &call.batch_id,
        request: &prepared.request,
        timeout: &prepared.timeout,
        max_retries: call.params.max_retries,
    }
}

// the five providers cancel_batch declares, each with the settings its request reads
fn cancel_target<'call>(
    call: &'call CancelBatchCall,
    prepared: &'call Prepared,
) -> Option<CancelTarget<'call>> {
    let values = cancel_values(call, prepared);
    match call.provider.0.as_str() {
        "openai" | "hosted_vllm" | "litellm_proxy" => {
            Some(CancelTarget::OpenAi(OpenAiCancelCall {
                values,
                api_base: prepared.settings.openai_base(&prepared.options),
                api_key: prepared.settings.openai_api_key(&prepared.options),
                organization: prepared.settings.openai_organization(&prepared.options),
            }))
        }
        "azure" => Some(CancelTarget::Azure(AzureCancelCall {
            values,
            api_base: prepared.settings.azure_base(&prepared.options),
            api_key: prepared.settings.azure_api_key(&prepared.options),
            api_version: prepared.settings.azure_version(&prepared.options),
            azure_ad_token: connection::azure_ad_token(&call.params, &prepared.settings),
        })),
        "vertex_ai" => Some(CancelTarget::Vertex(VertexCancelCall {
            values,
            api_base: prepared.options.api_base.as_deref(),
            vertex_project: prepared.settings.vertex_project(&prepared.options),
            vertex_location: prepared.settings.vertex_location(&prepared.options),
            vertex_credentials: prepared.settings.vertex_credentials(&prepared.options),
        })),
        "xai" => Some(CancelTarget::Xai(XaiCancelCall {
            values,
            api_base: prepared.options.api_base.as_deref(),
            api_key: prepared.options.api_key.as_ref(),
        })),
        "bedrock" => Some(CancelTarget::Bedrock(BedrockCancelCall {
            values,
            params: &call.params,
        })),
        _ => None,
    }
}

fn effective_model(call: &CancelBatchCall) -> Option<String> {
    match &call.call {
        CancelCall::Sync => call.model.clone(),
        CancelCall::Async { model_in_kwargs } => {
            model_in_kwargs.clone().or_else(|| call.model.clone())
        }
    }
}

fn check_cancel_limits(limits: &ApiLimits, call: &CancelBatchCall) -> Result<(), ApiLimitError> {
    let headers = call.extra_headers.as_ref().map_or(0, IndexMap::len);
    let body = call.extra_body.as_ref().map_or(0, IndexMap::len);
    limits.check(ApiBoundariesInput::Items(ItemsBoundary {
        known_count: headers.saturating_add(body),
    }))?;
    let extra_bytes = call
        .extra_headers
        .as_ref()
        .map_or(0, text_bytes)
        .saturating_add(call.extra_body.as_ref().map_or(0, text_bytes));
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: call.batch_id.len().saturating_add(extra_bytes),
    }))?;
    Ok(())
}

fn text_bytes(entries: &IndexMap<String, String>) -> usize {
    entries
        .iter()
        .map(|(name, value)| name.len().saturating_add(value.len()))
        .fold(0, usize::saturating_add)
}
