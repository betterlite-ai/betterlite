use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::SapModelError;

const CONTENT_MESSAGE: &str = "Content must be a string";

// LiteLLM's validate_different_content, a field validator that runs before the string type check.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "Json")]
pub struct MessageContent(String);

impl MessageContent {
    fn from_map(object: &Object) -> Result<Self, SapModelError> {
        if object.is_empty() {
            return Ok(Self(String::new()));
        }
        match object.get("text") {
            Some(Json::Text(text)) => Ok(Self(text.clone())),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => Err(SapModelError::Rejected {
                message: String::from(CONTENT_MESSAGE),
            }),
        }
    }

    fn from_list(items: &[Json]) -> Result<Self, SapModelError> {
        let mut parts: Vec<&str> = Vec::new();
        for item in items {
            match item {
                Json::Text(text) => parts.push(text),
                Json::Object(object) => match object.get("text") {
                    Some(Json::Text(text)) if !text.is_empty() => parts.push(text),
                    Some(Json::Text(_)) | None => {}
                    Some(
                        Json::Null
                        | Json::Bool(_)
                        | Json::Number(_)
                        | Json::List(_)
                        | Json::Object(_),
                    ) => {
                        return Err(SapModelError::Rejected {
                            message: String::from(CONTENT_MESSAGE),
                        });
                    }
                },
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
            }
        }
        Ok(Self(parts.join("\n")))
    }
}

impl TryFrom<Json> for MessageContent {
    type Error = SapModelError;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        match value {
            Json::Text(text) => Ok(Self(text)),
            Json::Object(object) => Self::from_map(&object),
            Json::List(items) => Self::from_list(&items),
            Json::Null | Json::Bool(_) | Json::Number(_) => Err(SapModelError::Rejected {
                message: String::from(CONTENT_MESSAGE),
            }),
        }
    }
}
