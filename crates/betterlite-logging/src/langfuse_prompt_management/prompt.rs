use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{AllMessageValues, PromptParams};
use indexmap::IndexMap;

use crate::langfuse_rest::records::{Prompt, PromptMeta};

use super::LangfusePromptManagementError;
use super::template;

// LangfusePromptManagement._compile_prompt: a text prompt becomes the system message, a chat prompt its list
pub(crate) fn compile_prompt(
    prompt: &Prompt,
    variables: Option<&PromptParams>,
) -> Result<Vec<AllMessageValues>, LangfusePromptManagementError> {
    let empty = PromptParams::new();
    let variables = variables.unwrap_or(&empty);
    match prompt {
        Prompt::Text(text) => {
            let content = template::compile_template(&text.prompt, variables);
            Ok(vec![Json::Object(IndexMap::from([
                ("role".to_owned(), Json::Text("system".to_owned())),
                ("content".to_owned(), Json::Text(content)),
            ]))])
        }
        Prompt::Chat(chat) => template::compile_chat_prompt(&chat.prompt, variables),
    }
}

// _compile_prompt_helper's langfuse_prompt_client.config.get("model")
pub(crate) fn template_model(
    prompt: &Prompt,
) -> Result<Option<String>, LangfusePromptManagementError> {
    let config = &meta(prompt).config;
    if matches!(config, Json::Null) {
        return Ok(None);
    }
    let Json::Object(fields) = config else {
        return Err(LangfusePromptManagementError::ConfigType);
    };
    match fields.get("model") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(model)) => Ok(Some(model.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangfusePromptManagementError::ModelType)
        }
    }
}

// LangfusePromptManagement._get_optional_params_from_langfuse keeps every config key but "model"
pub(crate) fn optional_params_from_langfuse(
    prompt: &Prompt,
) -> Result<PromptParams, LangfusePromptManagementError> {
    let config = &meta(prompt).config;
    if matches!(config, Json::Null) {
        return Ok(PromptParams::new());
    }
    let Json::Object(fields) = config else {
        return Err(LangfusePromptManagementError::ConfigType);
    };
    let mut params = fields.clone();
    params.shift_remove("model");
    Ok(params)
}

fn meta(prompt: &Prompt) -> &PromptMeta {
    match prompt {
        Prompt::Chat(chat) => &chat.meta,
        Prompt::Text(text) => &text.meta,
    }
}
