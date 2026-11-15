use betterlite_models::prompt_spec::{PromptInfo, PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::registry::prompt_registry_helpers::{
    DEFAULT_PROMPT_ENVIRONMENT, get_base_prompt_id, get_version_number,
    prompt_environment_or_default,
};

use super::rows::{PromptRow, PromptRowJson};

#[derive(Debug, Error)]
pub enum PromptRowConversionError {
    #[error("a stored prompt field is not valid prompt JSON")]
    InvalidStoredJson {
        #[source]
        source: serde_json::Error,
    },
}

fn parse_stored<T>(stored: &PromptRowJson) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    match stored {
        PromptRowJson::Text(text) => serde_json::from_str(text),
        PromptRowJson::Object(object) => serde_json::from_str(&serde_json::to_string(object)?),
    }
}

#[must_use]
pub fn get_latest_prompt_versions(mut prompts: Vec<PromptSpec>) -> Vec<PromptSpec> {
    prompts.sort_by_key(|prompt| get_version_number(&prompt.prompt_id));
    let mut latest_prompts: IndexMap<(String, String), PromptSpec> = IndexMap::new();
    for prompt in prompts {
        let environment = prompt_environment_or_default(Some(prompt.environment.as_str()));
        latest_prompts.insert((get_base_prompt_id(&prompt.prompt_id), environment), prompt);
    }
    latest_prompts.into_values().collect()
}

pub fn create_versioned_prompt_spec(
    db_prompt: &PromptRow,
) -> Result<PromptSpec, PromptRowConversionError> {
    let litellm_params = parse_stored::<PromptLiteLLMParams>(&db_prompt.litellm_params)
        .map_err(|source| PromptRowConversionError::InvalidStoredJson { source })?;
    let prompt_info = match db_prompt.prompt_info.as_ref() {
        Some(stored_info) => parse_stored::<PromptInfo>(stored_info)
            .map_err(|source| PromptRowConversionError::InvalidStoredJson { source })?,
        None => PromptInfo {
            prompt_type: "db".to_owned(),
            environment: DEFAULT_PROMPT_ENVIRONMENT.to_owned(),
            extra: IndexMap::new(),
        },
    };
    Ok(PromptSpec {
        prompt_id: format!("{}.v{}", db_prompt.prompt_id, db_prompt.version),
        litellm_params,
        prompt_info,
        created_at: Some(db_prompt.created_at),
        updated_at: Some(db_prompt.updated_at),
        version: Some(db_prompt.version),
        environment: db_prompt.environment.clone(),
        created_by: db_prompt.created_by.clone(),
    })
}
