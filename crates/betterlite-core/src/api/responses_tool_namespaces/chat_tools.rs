use betterlite_models::decisions::Json;
use serde::Serialize;

use super::error::NamespaceToolsError;
use super::values::Fields;
use crate::api::responses_codex_tools::custom_tools::FunctionTool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatFunction {
    pub name: String,
    pub description: String,
    pub parameters: Fields,
    pub strict: Json,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ToolExtras {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_callers: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_examples: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eager_input_streaming: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatFunctionTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: ChatFunction,
    #[serde(flatten)]
    pub extras: ToolExtras,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ChatTool {
    Function(Box<ChatFunctionTool>),
    Custom(FunctionTool),
    Passthrough(Fields),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebSearchOptions {
    pub search_context_size: Json,
    pub user_location: Json,
}

// a parameters schema always names its type
#[must_use]
pub fn with_object_type(mut parameters: Fields) -> Fields {
    if !parameters.contains_key("type") {
        parameters.insert("type".to_owned(), Json::Text("object".to_owned()));
    }
    parameters
}

// an absent or null parameters value is an empty schema; any other non-object is the caller's mistake
pub fn schema_parameters(value: Option<&Json>) -> Result<Fields, NamespaceToolsError> {
    match value {
        None | Some(Json::Null) => Ok(with_object_type(Fields::new())),
        Some(Json::Object(fields)) => Ok(with_object_type(fields.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(NamespaceToolsError::WrongType {
                field: "parameters",
                expected: "an object",
            })
        }
    }
}
