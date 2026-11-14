use std::error::Error;
use std::sync::Arc;

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::BatchObject;
use indexmap::IndexMap;

use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary, JsonBoundary,
};

use super::async_invoke;
use super::connection;
use super::{
    AzureCancelCall, BatchCancelDispatch, BatchProviderCancelError, BatchProviderCancelInput,
    BatchProviderCancelResult, BedrockCancelCall, CancelBatchCall, CancelCall, CancelProvider,
    OpenAiCancelCall, VertexCancelCall, XaiCancelCall,
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
    timeout: connection::CancelTimeout,
}

impl<Dispatch> BatchProviderCancel<Dispatch>
where
    Dispatch: BatchCancelDispatch,
    Dispatch::Error: Error + Send + Sync + 'static,
{
    #[must_use]
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
        };
        let batch = self.dispatch_branch(call, &prepared).await?;
        Ok(BatchProviderCancelResult {
            batch,
            model: effective_model(call),
        })
    }

    async fn dispatch_branch(
        &self,
        call: &CancelBatchCall,
        prepared: &Prepared,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        match &call.provider {
            CancelProvider::Openai | CancelProvider::HostedVllm | CancelProvider::LitellmProxy => {
                self.openai_branch(call, prepared).await
            }
            CancelProvider::Azure => self.azure_branch(call, prepared).await,
            CancelProvider::VertexAi => self.vertex_branch(call, prepared).await,
            CancelProvider::Xai => self.xai_branch(call, prepared).await,
            CancelProvider::Bedrock => self.bedrock_branch(call).await,
            CancelProvider::Other(provider) => Err(BatchProviderCancelError::Unsupported {
                provider: provider.clone(),
            }),
        }
    }

    async fn openai_branch(
        &self,
        call: &CancelBatchCall,
        prepared: &Prepared,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        let api_base = connection::openai_base(&call.params, &self.settings);
        self.dispatch
            .openai(OpenAiCancelCall {
                is_async: prepared.is_async,
                api_base: &api_base,
                api_key: connection::openai_api_key(&call.params, &self.settings),
                organization: connection::openai_organization(&call.params, &self.settings),
                request: &prepared.request,
                timeout: &prepared.timeout,
                max_retries: call.params.max_retries,
            })
            .await
            .map_err(BatchProviderCancelError::Dispatch)
    }

    async fn azure_branch(
        &self,
        call: &CancelBatchCall,
        prepared: &Prepared,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        self.dispatch
            .azure(AzureCancelCall {
                is_async: prepared.is_async,
                api_base: connection::azure_base(&call.params, &self.settings),
                api_key: connection::azure_api_key(&call.params, &self.settings),
                api_version: connection::azure_version(&call.params, &self.settings),
                azure_ad_token: call.params.azure_ad_token.as_ref(),
                request: &prepared.request,
                timeout: &prepared.timeout,
                max_retries: call.params.max_retries,
            })
            .await
            .map_err(BatchProviderCancelError::Dispatch)
    }

    async fn vertex_branch(
        &self,
        call: &CancelBatchCall,
        prepared: &Prepared,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        self.dispatch
            .vertex(VertexCancelCall {
                is_async: prepared.is_async,
                batch_id: &call.batch_id,
                api_base: call.params.api_base.as_deref(),
                vertex_project: connection::vertex_project(&call.params, &self.settings),
                vertex_location: connection::vertex_location(&call.params, &self.settings),
                vertex_credentials: connection::vertex_credentials(&call.params, &self.settings),
                timeout: &prepared.timeout,
                max_retries: call.params.max_retries,
            })
            .await
            .map_err(BatchProviderCancelError::Dispatch)
    }

    async fn xai_branch(
        &self,
        call: &CancelBatchCall,
        prepared: &Prepared,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        self.dispatch
            .xai(XaiCancelCall {
                is_async: prepared.is_async,
                batch_id: &call.batch_id,
                api_base: call.params.api_base.as_deref(),
                api_key: call.params.api_key.as_ref(),
                timeout: &prepared.timeout,
            })
            .await
            .map_err(BatchProviderCancelError::Dispatch)
    }

    async fn bedrock_branch(
        &self,
        call: &CancelBatchCall,
    ) -> Result<BatchObject, BatchProviderCancelError<Dispatch::Error>> {
        self.dispatch
            .bedrock(BedrockCancelCall {
                batch_id: &call.batch_id,
                params: &call.params,
            })
            .await
            .map_err(BatchProviderCancelError::Dispatch)
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
