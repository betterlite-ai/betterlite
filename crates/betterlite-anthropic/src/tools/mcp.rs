use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::ToolsError;
use super::fields::{non_null, string_list};
use super::wire::{Authorization, ToolConfiguration};

pub(super) fn tool_configuration(fields: &Object) -> Result<Option<ToolConfiguration>, ToolsError> {
    match non_null(fields.get("allowed_tools")) {
        None => Ok(None),
        Some(value) => Ok(Some(ToolConfiguration {
            allowed_tools: string_list(value)?,
        })),
    }
}

pub(super) fn authorization_token(fields: &Object) -> Result<Authorization, ToolsError> {
    let headers = match fields.get("headers") {
        None | Some(Json::Null) => return Ok(Authorization(None)),
        Some(Json::Object(headers)) => headers,
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "headers",
                expected: "an object",
            });
        }
    };
    match headers.get("Authorization") {
        None | Some(Json::Null) => Ok(Authorization(None)),
        Some(Json::Text(token)) => Ok(Authorization(Some(SecretString::from(
            token.replace("Bearer ", ""),
        )))),
        Some(_) => Err(ToolsError::WrongType {
            field: "headers.Authorization",
            expected: "a string",
        }),
    }
}
