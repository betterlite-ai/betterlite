use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::BatchObject;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, ItemsBoundary, JsonBoundary,
};

pub mod async_invoke;
pub mod connection;
mod tests;

use connection::{CancelParams, CancelTimeout};

// cancel_batch's provider union: the declared literal names plus the str branch that answers 400
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum CancelProvider {
    Openai,
    HostedVllm,
    LitellmProxy,
    Azure,
    VertexAi,
    Bedrock,
    Xai,
    Other(String),
}

impl CancelProvider {
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "openai" => Self::Openai,
            "hosted_vllm" => Self::HostedVllm,
            "litellm_proxy" => Self::LitellmProxy,
            "azure" => Self::Azure,
            "vertex_ai" => Self::VertexAi,
            "bedrock" => Self::Bedrock,
            "xai" => Self::Xai,
            other => Self::Other(other.to_owned()),
        }
    }

    #[must_use]
    pub const fn name(&self) -> &str {
        match self {
            Self::Openai => "openai",
            Self::HostedVllm => "hosted_vllm",
            Self::LitellmProxy => "litellm_proxy",
            Self::Azure => "azure",
            Self::VertexAi => "vertex_ai",
            Self::Bedrock => "bedrock",
            Self::Xai => "xai",
            Self::Other(name) => name.as_str(),
        }
    }
}

impl From<String> for CancelProvider {
    fn from(name: String) -> Self {
        Self::from_name(&name)
    }
}

// acancel_batch sets the async flag and lets a model in its kwargs replace the argument
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelCall {
    #[default]
    Sync,
    Async {
        model_in_kwargs: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelBatchCall {
    pub batch_id: String,
    #[serde(default)]
    pub model: Option<String>,
    pub provider: CancelProvider,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub params: CancelParams,
    #[serde(default)]
    pub call: CancelCall,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchProviderCancelInput {
    Cancel(Box<CancelBatchCall>),
    AsyncInvoke(async_invoke::AsyncInvokeStatus),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchProviderCancelResult {
    pub batch: BatchObject,
    // the model cancel_batch would hand provider resolution, after the async wrapper's kwarg wins
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelErrorFields {
    pub status: u16,
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: &'static str,
}

#[derive(Debug, Error)]
pub enum BatchProviderCancelError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error(transparent)]
    Timeout(#[from] connection::TimeoutValueError),
    #[error("LiteLLM doesn't support {provider} for 'cancel_batch'")]
    Unsupported { provider: String },
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
}

impl<DispatchError> BatchProviderCancelError<DispatchError>
where
    DispatchError: Error + 'static,
{
    #[must_use]
    pub const fn error_fields(&self) -> Option<CancelErrorFields> {
        match self {
            Self::Unsupported { .. } => Some(CancelErrorFields {
                status: 400,
                error_type: "bad_request_error",
                param: "None",
                code: "400",
            }),
            Self::Limits(_) | Self::Timeout(_) | Self::Dispatch(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OpenAiCancelCall<'call> {
    pub is_async: bool,
    pub api_base: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub organization: Option<&'call str>,
    pub request: &'call BatchIdRequest,
    pub timeout: &'call CancelTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct AzureCancelCall<'call> {
    pub is_async: bool,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub azure_ad_token: Option<&'call SecretString>,
    pub request: &'call BatchIdRequest,
    pub timeout: &'call CancelTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct VertexCancelCall<'call> {
    pub is_async: bool,
    pub batch_id: &'call str,
    pub api_base: Option<&'call str>,
    pub vertex_project: Option<&'call str>,
    pub vertex_location: Option<&'call str>,
    pub vertex_credentials: Option<&'call SecretString>,
    pub timeout: &'call CancelTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct XaiCancelCall<'call> {
    pub is_async: bool,
    pub batch_id: &'call str,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: &'call CancelTimeout,
}

// the source forwards its whole kwargs dict to the bedrock handler, which only names credential fields
#[derive(Debug, Clone, Copy)]
pub struct BedrockCancelCall<'call> {
    pub batch_id: &'call str,
    pub params: &'call CancelParams,
}

pub trait BatchCancelDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn openai(
        &self,
        call: OpenAiCancelCall<'_>,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;

    fn azure(
        &self,
        call: AzureCancelCall<'_>,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;

    fn vertex(
        &self,
        call: VertexCancelCall<'_>,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;

    fn xai(
        &self,
        call: XaiCancelCall<'_>,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;

    fn bedrock(
        &self,
        call: BedrockCancelCall<'_>,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;
}

pub struct BatchProviderCancel<Dispatch> {
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub dispatch: Dispatch,
    pub limits: ApiLimits,
    pub settings: Settings,
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
        let timeout = connection::resolve_timeout(call)?;
        let request = BatchIdRequest {
            batch_id: Some(call.batch_id.clone()),
            extra_headers: call.extra_headers.clone(),
            extra_body: call.extra_body.clone(),
            timeout: None,
        };
        let is_async = matches!(&call.call, CancelCall::Async { .. });
        let batch = match &call.provider {
            CancelProvider::Openai | CancelProvider::HostedVllm | CancelProvider::LitellmProxy => {
                let api_base = connection::openai_base(&call.params, &self.settings);
                self.dispatch
                    .openai(OpenAiCancelCall {
                        is_async,
                        api_base: &api_base,
                        api_key: connection::openai_api_key(&call.params, &self.settings),
                        organization: connection::openai_organization(&call.params, &self.settings),
                        request: &request,
                        timeout: &timeout,
                        max_retries: call.params.max_retries,
                    })
                    .await?
            }
            CancelProvider::Azure => {
                self.dispatch
                    .azure(AzureCancelCall {
                        is_async,
                        api_base: connection::azure_base(&call.params, &self.settings),
                        api_key: connection::azure_api_key(&call.params, &self.settings),
                        api_version: connection::azure_version(&call.params, &self.settings),
                        azure_ad_token: call.params.azure_ad_token.as_ref(),
                        request: &request,
                        timeout: &timeout,
                        max_retries: call.params.max_retries,
                    })
                    .await?
            }
            CancelProvider::VertexAi => {
                let api_base = call.params.api_base.as_deref();
                self.dispatch
                    .vertex(VertexCancelCall {
                        is_async,
                        batch_id: &call.batch_id,
                        api_base,
                        vertex_project: connection::vertex_project(&call.params, &self.settings),
                        vertex_location: connection::vertex_location(&call.params, &self.settings),
                        vertex_credentials: connection::vertex_credentials(
                            &call.params,
                            &self.settings,
                        ),
                        timeout: &timeout,
                        max_retries: call.params.max_retries,
                    })
                    .await?
            }
            CancelProvider::Xai => {
                self.dispatch
                    .xai(XaiCancelCall {
                        is_async,
                        batch_id: &call.batch_id,
                        api_base: call.params.api_base.as_deref(),
                        api_key: call.params.api_key.as_ref(),
                        timeout: &timeout,
                    })
                    .await?
            }
            CancelProvider::Bedrock => {
                self.dispatch
                    .bedrock(BedrockCancelCall {
                        batch_id: &call.batch_id,
                        params: &call.params,
                    })
                    .await?
            }
            CancelProvider::Other(provider) => {
                return Err(BatchProviderCancelError::Unsupported {
                    provider: provider.clone(),
                });
            }
        };
        Ok(BatchProviderCancelResult {
            batch,
            model: effective_model(call),
        })
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
    let headers = call.extra_headers.as_ref().map_or(0, BTreeMap::len);
    let body = call.extra_body.as_ref().map_or(0, BTreeMap::len);
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

fn text_bytes(entries: &BTreeMap<String, String>) -> usize {
    entries
        .iter()
        .map(|(name, value)| name.len().saturating_add(value.len()))
        .fold(0, usize::saturating_add)
}
