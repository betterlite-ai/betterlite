use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::PromptLiteLLMParams;
use serde::Serialize;

use crate::gitlab_client::config::GitLabConfig;

use super::super::error::InitializerError;

pub(crate) fn gitlab_config(
    litellm_params: &PromptLiteLLMParams,
) -> Result<GitLabConfig, InitializerError> {
    match litellm_params.extra.get("gitlab_config") {
        None | Some(ContractJson::Null) => Err(InitializerError::MissingConfig),
        Some(ContractJson::Object(fields)) if fields.is_empty() => {
            Err(InitializerError::MissingConfig)
        }
        Some(value) => parse_gitlab_config(value),
    }
}

pub(crate) fn git_ref(
    litellm_params: &PromptLiteLLMParams,
) -> Result<Option<String>, InitializerError> {
    let Some(value) = litellm_params
        .extra
        .get("git_ref")
        .filter(|value| **value != ContractJson::Null)
    else {
        return Ok(None);
    };
    match value {
        ContractJson::Text(name) => Ok(Some(name.clone())),
        ContractJson::Null
        | ContractJson::Bool(_)
        | ContractJson::Number(_)
        | ContractJson::List(_)
        | ContractJson::Object(_) => Err(InitializerError::RefNotString),
    }
}

pub(crate) fn parse_gitlab_config<Value>(value: &Value) -> Result<GitLabConfig, InitializerError>
where
    Value: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).map_err(InitializerError::Config)?)
        .map_err(InitializerError::Config)
}

mod tests;
