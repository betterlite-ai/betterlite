use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::fine_tuning_read::{ReadTimeout, ReadTimeoutValue};
use serde::{Deserialize, Serialize};

use super::super::OracleLimits;
use super::super::call_kind::CallKind;
use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    FileContent,
    AfileContent,
}

// the client kwarg: none, an OpenAI or Azure SDK client, or a LiteLLM HTTPHandler
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppliedKind {
    #[default]
    None,
    OpenaiSdk,
    AzureSdk,
    HttpHandler,
}

// one SDK call; api_base is always the mock's, and every other kwarg is passed only when the step names it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub function: Function,
    pub file_id: String,
    #[serde(default)]
    pub model: Option<String>,
    // the Python step declares the provider as text, so an unnamed one is omitted rather than sent as null
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub timeout: Option<ReadTimeout>,
    #[serde(default)]
    pub request_timeout: Option<ReadTimeoutValue>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    // an absent size is LiteLLM's 1048576 default
    #[serde(default)]
    pub chunk_size: Option<u64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub client: SuppliedKind,
    // the case hands over a real logging object, so the identity file_content_streaming writes is observable
    #[serde(default)]
    pub supplied_logging: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() || self.steps.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(format!(
                "a file content call needs 1 to {limits.max_requests} steps and has {}",
                self.steps.len()
            )));
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        super::send_steps(self, api_base, env)
    }
}
