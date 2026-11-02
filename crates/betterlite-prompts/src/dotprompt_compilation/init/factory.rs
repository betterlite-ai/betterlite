use std::path::PathBuf;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;
use thiserror::Error;

use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_templates::content::dotprompt_content_to_json;
use crate::dotprompt_templates::frontmatter::FrontmatterError;
use crate::dotprompt_templates::version::strip_version_suffix;

use super::manager::{DotpromptManager, PromptDataInput};

#[derive(Debug, Error)]
pub enum DotpromptInitializerError {
    #[error(
        "Cannot set prompt_directory when working with prompt_initializer. Needs to be a specific dotprompt file"
    )]
    PromptDirectorySet,
    #[error("litellm_params.{field} must be a string")]
    WrongType { field: String },
    #[error(transparent)]
    Content(#[from] FrontmatterError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn prompt_initializer(
    litellm_params: &PromptLiteLLMParams,
    prompt_spec: &PromptSpec,
    global_prompt_directory: &GlobalPromptDirectory,
) -> Result<DotpromptManager, DotpromptInitializerError> {
    let prompt_directory = extra_text(&litellm_params.extra, "prompt_directory")?;
    if prompt_directory.is_some() {
        return Err(DotpromptInitializerError::PromptDirectorySet);
    }

    let prompt_file = extra_text(&litellm_params.extra, "prompt_file")?.map(PathBuf::from);
    let explicit_prompt_data = extra_prompt_data(&litellm_params.extra)?;

    let prompt_data = match (
        &explicit_prompt_data,
        &prompt_file,
        &litellm_params.dotprompt_content,
    ) {
        (None, None, Some(dotprompt_content)) => Some(PromptDataInput::Object(
            dotprompt_content_to_json(dotprompt_content)?,
        )),
        _ => explicit_prompt_data,
    };

    let registration_prompt_id = litellm_params
        .prompt_id
        .clone()
        .or_else(|| strip_version_suffix(&prompt_spec.prompt_id).map(str::to_owned))
        .unwrap_or_else(|| prompt_spec.prompt_id.clone());

    DotpromptManager::new(
        None,
        prompt_file,
        prompt_data,
        Some(registration_prompt_id),
        global_prompt_directory,
    )
    .map_err(DotpromptInitializerError::from)
}

fn extra_field<'extra>(
    extra: &'extra IndexMap<String, ContractJson>,
    field: &str,
) -> Option<&'extra ContractJson> {
    match extra.get(field) {
        None | Some(ContractJson::Null) => None,
        Some(value) => Some(value),
    }
}

fn extra_text(
    extra: &IndexMap<String, ContractJson>,
    field: &str,
) -> Result<Option<String>, DotpromptInitializerError> {
    let Some(value) = extra_field(extra, field) else {
        return Ok(None);
    };
    if let ContractJson::Text(text) = value {
        Ok(Some(text.clone()))
    } else {
        Err(DotpromptInitializerError::WrongType {
            field: field.to_owned(),
        })
    }
}

fn extra_prompt_data(
    extra: &IndexMap<String, ContractJson>,
) -> Result<Option<PromptDataInput>, DotpromptInitializerError> {
    match extra_field(extra, "prompt_data") {
        None => Ok(None),
        Some(ContractJson::Text(text)) => Ok(Some(PromptDataInput::Text(text.clone()))),
        Some(ContractJson::Object(object)) => {
            Ok(Some(PromptDataInput::Object(from_contract_params(object))))
        }
        Some(_) => Err(DotpromptInitializerError::WrongType {
            field: "prompt_data".to_owned(),
        }),
    }
}

fn from_contract_params(object: &IndexMap<String, ContractJson>) -> Object {
    let mut result = Object::new();
    for (key, value) in object {
        result.insert(key.clone(), from_contract_json(value));
    }
    result
}

fn from_contract_json(value: &ContractJson) -> Json {
    match value {
        ContractJson::Null => Json::Null,
        ContractJson::Bool(flag) => Json::Bool(*flag),
        ContractJson::Number(number) => Json::Number(number.clone()),
        ContractJson::Text(text) => Json::Text(text.clone()),
        ContractJson::List(items) => Json::List(items.iter().map(from_contract_json).collect()),
        ContractJson::Object(fields) => Json::Object(from_contract_params(fields)),
    }
}

mod tests;
