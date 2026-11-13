use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::api::messages_content_primitives::tools::truncate_tool_name;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ToolChoiceError {
    #[error("tool_choice is not a JSON object")]
    Shape,
    #[error("tool_choice is missing its type")]
    MissingType,
    #[error("tool_choice.type is not any, auto, tool or none")]
    Unsupported,
    #[error("tool_choice.name is not a string")]
    Name,
}

pub fn tool_choice_to_openai(tool_choice: &Json) -> Result<Json, ToolChoiceError> {
    let Some(fields) = tool_choice.as_object() else {
        return Err(ToolChoiceError::Shape);
    };
    match fields.get("type") {
        None => Err(ToolChoiceError::MissingType),
        Some(Json::Text(kind)) => match kind.as_str() {
            "any" => Ok(Json::Text("required".to_owned())),
            "auto" => Ok(Json::Text("auto".to_owned())),
            "none" => Ok(Json::Text("none".to_owned())),
            "tool" => forced_tool(fields),
            _ => Err(ToolChoiceError::Unsupported),
        },
        Some(_) => Err(ToolChoiceError::Unsupported),
    }
}

fn forced_tool(fields: &Object) -> Result<Json, ToolChoiceError> {
    let name = match fields.get("name") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(name)) => name.clone(),
        Some(_) => return Err(ToolChoiceError::Name),
    };
    let mut function = Object::new();
    function.insert("name".to_owned(), Json::Text(truncate_tool_name(&name)));
    let mut choice = Object::new();
    choice.insert("type".to_owned(), Json::Text("function".to_owned()));
    choice.insert("function".to_owned(), Json::Object(function));
    Ok(Json::Object(choice))
}
