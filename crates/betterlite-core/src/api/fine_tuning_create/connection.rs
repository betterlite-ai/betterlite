use super::CreateFineTuningJobCall;
use crate::api::resource_sdk_settings::ResourceSdkRequestOptions;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, request_timeout,
    resolve_fine_tuning_timeout,
};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

// the kwargs create_fine_tuning_job reads through GenericLiteLLMParams and get_litellm_params
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CreateParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
    // the Azure-only root parameter the extra-body helper copies into extra_body
    #[serde(rename = "trainingType")]
    pub training_type: Option<Json>,
    pub timeout: Option<CreateTimeout>,
    pub request_timeout: Option<f64>,
    pub max_retries: Option<i64>,
}
// GenericLiteLLMParams.timeout declares float | str | httpx.Timeout
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CreateTimeout {
    Plain(TimeoutValue),
    Structured(StructuredTimeout),
}
impl From<&CreateParams> for ResourceSdkRequestOptions {
    fn from(params: &CreateParams) -> Self {
        Self {
            api_base: params.api_base.clone(),
            api_version: params.api_version.clone(),
            api_key: params.api_key.clone(),
            organization: params.organization.clone(),
            vertex_project: params.vertex_project.clone(),
            vertex_location: params.vertex_location.clone(),
            vertex_credentials: params.vertex_credentials.clone(),
        }
    }
}
// the source resolves the timeout before it branches, so an invalid value fails even for an unsupported provider
pub fn resolve_timeout(call: &CreateFineTuningJobCall) -> Result<FineTuningTimeout, TimeoutError> {
    let provider = call.custom_llm_provider.as_str();
    match call.params.timeout.as_ref() {
        Some(CreateTimeout::Structured(timeout)) => {
            resolve_fine_tuning_timeout(None, provider, Some(*timeout))
        }
        Some(CreateTimeout::Plain(value)) if !is_unset(value) => {
            resolve_fine_tuning_timeout(Some(value), provider, None)
        }
        // optional_params.timeout or request_timeout or 600, where a zero or an empty value falls through
        None | Some(CreateTimeout::Plain(_)) => Ok(FineTuningTimeout::Seconds(request_timeout(
            None,
            call.params.request_timeout,
        )?)),
    }
}
fn is_unset(value: &TimeoutValue) -> bool {
    match value {
        TimeoutValue::Number(seconds) => *seconds == 0.0,
        TimeoutValue::String(text) => text.is_empty(),
    }
}
