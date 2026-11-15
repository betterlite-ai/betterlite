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

// cancel_batch's provider is the declared literal names plus any other string, which answers 400
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct CancelProvider(pub String);

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

// the values every provider branch reads after the shared limit and timeout checks
#[derive(Debug, Clone, Copy)]
pub struct CancelValues<'call> {
    pub is_async: bool,
    pub batch_id: &'call str,
    pub request: &'call BatchIdRequest,
    pub timeout: &'call FineTuningTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct OpenAiCancelCall<'call> {
    pub values: CancelValues<'call>,
    // the openai branch always has a base: the call's, the settings' or the default
    pub api_base: String,
    pub api_key: Option<&'call SecretString>,
    pub organization: Option<&'call str>,
}

#[derive(Debug, Clone, Copy)]
pub struct AzureCancelCall<'call> {
    pub values: CancelValues<'call>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub azure_ad_token: Option<&'call SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct VertexCancelCall<'call> {
    pub values: CancelValues<'call>,
    pub api_base: Option<&'call str>,
    pub vertex_project: Option<&'call str>,
    pub vertex_location: Option<&'call str>,
    pub vertex_credentials: Option<&'call SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct XaiCancelCall<'call> {
    pub values: CancelValues<'call>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
}

// the source forwards its whole kwargs dict to the bedrock handler, which only names credential fields
#[derive(Debug, Clone, Copy)]
pub struct BedrockCancelCall<'call> {
    pub values: CancelValues<'call>,
    pub params: &'call CancelParams,
}

// cancel_batch's five branches; the provider's wire name decides which one a call takes
#[derive(Debug, Clone)]
pub enum CancelTarget<'call> {
    OpenAi(OpenAiCancelCall<'call>),
    Azure(AzureCancelCall<'call>),
    Vertex(VertexCancelCall<'call>),
    Xai(XaiCancelCall<'call>),
    Bedrock(BedrockCancelCall<'call>),
}

pub trait BatchCancelDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn cancel(
        &self,
        target: CancelTarget<'_>,
    ) -> impl Future<Output = Result<BatchObject, Self::Error>> + Send;
}
