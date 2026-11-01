use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::{PromptManagementClient, PromptParams};
use thiserror::Error;

use crate::arguments::json::{Json, Object};
use crate::dotprompt_compilation::init::manager::{DotpromptManager, DotpromptManagerError};
use crate::dotprompt_compilation::messages;
use crate::dotprompt_templates::render::RenderError;

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("prompt_id is required for dotprompt manager")]
    PromptIdRequired,
    #[error("Error compiling prompt '{prompt_id}': {source}")]
    Compile {
        prompt_id: String,
        #[source]
        source: CompileFailure,
    },
}

#[derive(Debug, Error)]
pub enum CompileFailure {
    #[error(transparent)]
    Manager(#[from] DotpromptManagerError),
    #[error("Prompt '{prompt_id}'{version_suffix} not found in prompt directory")]
    NotFound {
        prompt_id: String,
        version_suffix: String,
    },
    #[error(transparent)]
    Render(#[from] RenderError),
}

pub(super) fn compile_inner(
    manager: &DotpromptManager,
    prompt_id: &str,
    prompt_variables: Option<&Object>,
    version: Option<i64>,
) -> Result<PromptManagementClient, CompileFailure> {
    let template_fields = manager.with_prompt_manager(|prompt_manager| {
        prompt_manager
            .get_prompt(prompt_id, version)
            .map(|template| (template.model.clone(), template.optional_params.clone()))
    })?;

    let Some((template_model, optional_params)) = template_fields else {
        // Python's truthiness check treats an explicit version of 0 the same as no version.
        let version_suffix = match version {
            Some(requested) if requested != 0 => format!(" (version {requested})"),
            _ => String::new(),
        };
        return Err(CompileFailure::NotFound {
            prompt_id: prompt_id.to_owned(),
            version_suffix,
        });
    };

    let rendered_content = manager.with_prompt_manager(|prompt_manager| {
        prompt_manager.render(prompt_id, prompt_variables, version)
    })??;

    let messages = messages::convert_to_messages(&rendered_content);

    Ok(PromptManagementClient {
        prompt_id: Some(prompt_id.to_owned()),
        prompt_template: messages,
        prompt_template_model: template_model,
        prompt_optional_params: Some(to_contract_params(&optional_params)),
        completed_messages: None,
    })
}

fn to_contract_params(object: &Object) -> PromptParams {
    let mut result = PromptParams::new();
    for (key, value) in object {
        result.insert(key.clone(), to_contract_json(value));
    }
    result
}

fn to_contract_json(value: &Json) -> ContractJson {
    match value {
        Json::Null => ContractJson::Null,
        Json::Bool(flag) => ContractJson::Bool(*flag),
        Json::Number(number) => ContractJson::Number(number.clone()),
        Json::Text(text) => ContractJson::Text(text.clone()),
        Json::List(items) => ContractJson::List(items.iter().map(to_contract_json).collect()),
        Json::Object(fields) => ContractJson::Object(to_contract_params(fields)),
    }
}

mod tests;
