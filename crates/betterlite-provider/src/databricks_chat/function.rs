use betterlite_prompts::arguments::json::{Json, Object};

use crate::databricks_chat::error::DatabricksChatError;
use crate::databricks_chat::wire::render;

// The parts of LiteLLM's Function a reply or a delta tool call carries
pub(crate) struct NormalizedFunction {
    pub arguments: String,
    pub name: Option<String>,
}

// Function(**function): a null arguments value falls back to a dict parameters value
pub(crate) fn normalize_function(
    function: &Object,
) -> Result<NormalizedFunction, DatabricksChatError> {
    Ok(NormalizedFunction {
        arguments: arguments_of(function)?,
        name: name_of(function)?,
    })
}

fn arguments_of(function: &Object) -> Result<String, DatabricksChatError> {
    match function.get("arguments") {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Object(fields)) => Ok(render(&Json::Object(fields.clone()))),
        None | Some(Json::Null) => match function.get("parameters") {
            Some(Json::Object(fields)) => Ok(render(&Json::Object(fields.clone()))),
            None | Some(_) => Ok(String::new()),
        },
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => Err(DatabricksChatError::Shape {
            message: String::from(
                "the arguments of a tool call function must be a string or an object",
            ),
        }),
    }
}

fn name_of(function: &Object) -> Result<Option<String>, DatabricksChatError> {
    match function.get("name") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(DatabricksChatError::Shape {
                message: String::from("the name of a tool call function must be a string"),
            })
        }
    }
}
