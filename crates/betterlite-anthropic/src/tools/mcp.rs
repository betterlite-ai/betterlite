use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::ToolsError;
use super::fields::{non_null, string_list};
use super::wire::ToolConfiguration;

pub(super) fn tool_configuration(fields: &Object) -> Result<Option<ToolConfiguration>, ToolsError> {
    match non_null(fields.get("allowed_tools")) {
        None => Ok(None),
        Some(value) => Ok(Some(ToolConfiguration {
            allowed_tools: string_list(value, "allowed_tools")?,
        })),
    }
}

pub(super) fn authorization_token(fields: &Object) -> Result<Option<SecretString>, ToolsError> {
    let headers = match fields.get("headers") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(headers)) => headers,
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "headers",
                expected: "an object",
            });
        }
    };
    match headers.get("Authorization") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(token)) => Ok(Some(SecretString::from(token.replace("Bearer ", "")))),
        Some(_) => Err(ToolsError::WrongType {
            field: "headers.Authorization",
            expected: "a string",
        }),
    }
}
