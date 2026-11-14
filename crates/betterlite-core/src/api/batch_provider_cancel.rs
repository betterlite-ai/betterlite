use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::BatchObject;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

pub mod async_invoke;
pub mod connection;
pub mod service;
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
