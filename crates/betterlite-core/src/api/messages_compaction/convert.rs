use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::arguments::json::Object as PromptObject;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::summary_history::system::SystemPrompt;

// the prompts crate owns a second Json and exposes only the prompts-to-wire direction, so the reverse is walked here
pub fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(prompt_object(fields)),
    }
}

fn prompt_object(fields: &IndexMap<String, Json>) -> PromptObject {
    let mut object = PromptObject::new();
    for (key, value) in fields {
        object.insert(key.clone(), prompt_json(value));
    }
    object
}

#[derive(Debug, Error)]
#[error("the context edit's system prompt is neither text nor a list of blocks")]
pub struct SystemShapeError;

// system is str | list[dict] | None in the dispatcher's signature; a non-string, non-list value names itself
pub fn system_prompt(system: Option<&Json>) -> Result<Option<SystemPrompt>, SystemShapeError> {
    match system {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(SystemPrompt::Text(text.clone()))),
        Some(Json::List(items)) => Ok(Some(SystemPrompt::Blocks(
            items.iter().map(prompt_json).collect(),
        ))),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Err(SystemShapeError),
    }
}
