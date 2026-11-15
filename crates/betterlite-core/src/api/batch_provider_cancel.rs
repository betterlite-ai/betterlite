use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::BatchObject;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, TimeoutError};

pub mod async_invoke;
pub mod connection;
pub mod service;
mod tests;

use connection::CancelParams;

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

    // supports_httpx_timeout reads this name: only openai, azure and bedrock keep a structured timeout
    #[must_use]
    pub fn wire_name(&self) -> &str {
        match self {
            Self::Openai => "openai",
            Self::HostedVllm => "hosted_vllm",
            Self::LitellmProxy => "litellm_proxy",
            Self::Azure => "azure",
            Self::VertexAi => "vertex_ai",
            Self::Bedrock => "bedrock",
            Self::Xai => "xai",
            Self::Other(name) => name,
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
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
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

// litellm's BadRequestError carries the status alone: openai's APIError leaves type, param and code null
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelErrorFields {
    pub status: u16,
    pub error_type: Option<&'static str>,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
}

#[derive(Debug, Error)]
pub enum BatchProviderCancelError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("LiteLLM doesn't support {provider} for 'cancel_batch'")]
    Unsupported { provider: String },
    #[error(transparent)]
    Dispatch(DispatchError),
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
                error_type: None,
                param: None,
                code: None,
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
    pub timeout: &'call FineTuningTimeout,
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
    pub timeout: &'call FineTuningTimeout,
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
    pub timeout: &'call FineTuningTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct XaiCancelCall<'call> {
    pub is_async: bool,
    pub batch_id: &'call str,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: &'call FineTuningTimeout,
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
