use std::collections::BTreeMap;

use betterlite_bedrock::sagemaker_runtime::endpoint::ParamMap;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::EnterpriseError;

// the _CompletionDispatchContext values LiteLLM's _complete_sagemaker reads outside the shared call
#[derive(Clone, Copy, Default)]
pub struct SagemakerRequest<'request> {
    pub custom_prompt_dict: Option<&'request BTreeMap<String, CustomPromptEntry>>,
    pub global_custom_prompt_dict: Option<&'request BTreeMap<String, CustomPromptEntry>>,
    pub hf_model_name: Option<&'request str>,
}

pub fn convert<T, U>(value: &T) -> Result<U, EnterpriseError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|source| EnterpriseError::Encode { source })
}

// B5840 reads the AWS values and the region out of the call's optional params, so the map keeps every key as raw JSON
pub fn signing_params(optional_params: &Object) -> Result<ParamMap, EnterpriseError> {
    optional_params
        .iter()
        .map(|(name, value)| {
            serde_json::value::to_raw_value(value)
                .map(|raw| (name.clone(), raw))
                .map_err(|source| EnterpriseError::Encode { source })
        })
        .collect()
}
