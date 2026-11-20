use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::error::FieldError;
use super::records::Params;

pub(super) fn ssl_verify(params: &Params) -> Result<Option<bool>, FieldError> {
    match params.get("ssl_verify") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(FieldError::Type {
            field: "ssl_verify",
            expected: "a boolean",
        }),
    }
}

pub(super) fn api_base(params: &Params) -> Result<Option<&str>, FieldError> {
    match params.get("api_base") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value)),
        Some(_) => Err(FieldError::Type {
            field: "api_base",
            expected: "a string",
        }),
    }
}

pub(super) fn api_key(params: &Params) -> Result<Option<SecretString>, FieldError> {
    match params.get("api_key") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(SecretString::from(value.clone()))),
        Some(_) => Err(FieldError::Type {
            field: "api_key",
            expected: "a string",
        }),
    }
}
