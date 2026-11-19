use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use indexmap::IndexMap;
use secrecy::zeroize::Zeroize;
use secrecy::{CloneableSecret, ExposeSecret as _, SecretBox, SecretString, SerializableSecret};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use thiserror::Error;

use crate::api::resource_sdk_settings::ResourceSdkRequestOptions;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue,
};

pub mod page;
pub mod service;
mod tests;

// the provider literal both calls declare; absent is openai, and any other name is rejected later
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReadProvider(pub String);

// the field defaults through this type so an absent key takes openai and an explicit null stays None
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReadProviderField(pub Option<ReadProvider>);

impl Default for ReadProviderField {
    fn default() -> Self {
        Self(Some(ReadProvider("openai".to_owned())))
    }
}

// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout, and only a str is converted
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ReadTimeout {
    Plain(ReadTimeoutValue),
    Structured(ReadStructuredTimeout),
}

// a float stays serde_json's Number because the workspace's arbitrary_precision loses it under flatten
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ReadTimeoutValue {
    Number(Number),
    Text(String),
}

impl TryFrom<&ReadTimeoutValue> for TimeoutValue {
    type Error = TimeoutError;

    fn try_from(value: &ReadTimeoutValue) -> Result<Self, Self::Error> {
        match value {
            ReadTimeoutValue::Number(number) => Ok(Self::Number(
                number.as_f64().ok_or(TimeoutError::NonFinite)?,
            )),
            ReadTimeoutValue::Text(text) => Ok(Self::String(text.clone())),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReadStructuredTimeout {
    pub connect: Option<Number>,
    pub read: Option<Number>,
    pub write: Option<Number>,
    pub pool: Option<Number>,
}

impl TryFrom<&ReadStructuredTimeout> for StructuredTimeout {
    type Error = TimeoutError;

    fn try_from(timeout: &ReadStructuredTimeout) -> Result<Self, Self::Error> {
        Ok(Self {
            connect: optional_seconds(timeout.connect.as_ref())?,
            read: optional_seconds(timeout.read.as_ref())?,
            write: optional_seconds(timeout.write.as_ref())?,
            pool: optional_seconds(timeout.pool.as_ref())?,
        })
    }
}

// a buffered number reaches no finite f64 only when it is too large for one, which a timeout rejects
fn optional_seconds(number: Option<&Number>) -> Result<Option<f64>, TimeoutError> {
    number
        .map(|value| value.as_f64().ok_or(TimeoutError::NonFinite))
        .transpose()
}

// the key a case or the Python child carries as text, kept out of Debug by the SecretBox around it
#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ApiKeyText(String);

impl Zeroize for ApiKeyText {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl CloneableSecret for ApiKeyText {}

impl SerializableSecret for ApiKeyText {}

// the kwargs both calls read through GenericLiteLLMParams, beside request_timeout from kwargs
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct FineTuningReadParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretBox<ApiKeyText>>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub timeout: Option<ReadTimeout>,
    pub request_timeout: Option<ReadTimeoutValue>,
    pub max_retries: Option<i64>,
}

impl From<&FineTuningReadParams> for ResourceSdkRequestOptions {
    fn from(params: &FineTuningReadParams) -> Self {
        Self {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.expose_secret().0.clone())),
            organization: params.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            vertex_credentials: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FineTuningListCall {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub provider: ReadProviderField,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    // LiteLLM forwards extra_body as the caller's own JSON, so a non-string value still parses
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub params: FineTuningReadParams,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FineTuningRetrieveCall {
    pub fine_tuning_job_id: String,
    #[serde(default)]
    pub provider: ReadProviderField,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub params: FineTuningReadParams,
}

impl FineTuningListCall {
    fn operation(&self) -> ReadOperation<'_> {
        ReadOperation::List {
            after: self.after.as_deref(),
            limit: self.limit,
        }
    }
}

impl FineTuningRetrieveCall {
    fn operation(&self) -> ReadOperation<'_> {
        ReadOperation::Retrieve {
            job_id: &self.fine_tuning_job_id,
        }
    }
}

// the four functions differ in their async flag and retrieval id; list is not decorated, retrieve is
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FineTuningReadInput {
    List(Box<FineTuningListCall>),
    #[serde(rename = "alist")]
    AList(Box<FineTuningListCall>),
    Retrieve(Box<FineTuningRetrieveCall>),
    #[serde(rename = "aretrieve")]
    ARetrieve(Box<FineTuningRetrieveCall>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FineTuningReadOutcome {
    Page(page::FineTuningPage),
    Job(Box<FineTuneJob>),
}

// list hands back the SDK page untouched, so only retrieve decodes and normalizes a job
#[derive(Debug, Clone, Copy)]
pub enum ReadOperation<'call> {
    List {
        after: Option<&'call str>,
        limit: Option<i64>,
    },
    Retrieve {
        job_id: &'call str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ReadValues<'call> {
    pub is_async: bool,
    pub operation: ReadOperation<'call>,
    pub timeout: &'call FineTuningTimeout,
    pub max_retries: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct OpenAiReadCall<'call> {
    pub values: ReadValues<'call>,
    // the openai branch always has a base: the call's, the settings' or the default
    pub api_base: String,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub organization: Option<&'call str>,
}

#[derive(Debug, Clone, Copy)]
pub struct AzureReadCall<'call> {
    pub values: ReadValues<'call>,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_version: Option<&'call str>,
    pub organization: Option<&'call str>,
}

// both read calls have exactly the openai and azure branches; any other name answers 400 first
#[derive(Debug, Clone)]
pub enum ReadTarget<'call> {
    OpenAi(OpenAiReadCall<'call>),
    Azure(AzureReadCall<'call>),
}

// the dispatch answers the provider's bytes; the list page and the single job are decoded here
pub trait FineTuningReadDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn read(
        &self,
        target: ReadTarget<'_>,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;
}

// an unsupported provider answers a 400 with null type, param and code; a bad timeout is a ValueError
#[derive(Debug, Error)]
pub enum FineTuningReadError<DispatchError = Infallible>
where
    DispatchError: Error + 'static,
{
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("LiteLLM doesn't support {provider} for 'create_batch'. Only 'openai' is supported.")]
    UnsupportedList { provider: String },
    #[error(
        "LiteLLM doesn't support {provider} for 'retrieve_fine_tuning_job'. Only 'openai' and 'azure' are supported."
    )]
    UnsupportedRetrieve { provider: String },
    #[error(transparent)]
    Decode(#[from] FineTuneError),
    #[error("the fine-tuning page is not valid: {source}")]
    Page {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Dispatch(DispatchError),
}
