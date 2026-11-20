use betterlite_models::decisions::Json;

use super::config::Params;
use super::error::SearchError;

// litellm_params is a typed mapping, so a key of the wrong type is an error that names the field
pub(super) fn text<'params>(
    params: &'params Params,
    name: &'static str,
) -> Result<Option<&'params str>, SearchError> {
    match params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(SearchError::Field {
                field: name,
                expected: "a string",
            })
        }
    }
}

pub(super) fn ssl_verify(params: &Params) -> Result<Option<bool>, SearchError> {
    match params.get("ssl_verify") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(SearchError::Field {
                field: "ssl_verify",
                expected: "a boolean",
            })
        }
    }
}

// dict.get(key, None) reads an explicit null as the absent value
pub(super) const fn present(json: &Json) -> Option<&Json> {
    match json {
        Json::Null => None,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Some(json)
        }
    }
}

pub(super) fn integer(json: &Json, name: &'static str) -> Result<Option<i64>, SearchError> {
    if matches!(json, Json::Null) {
        return Ok(None);
    }
    if let Json::Number(number) = json {
        return number.as_i64().map(Some).ok_or(SearchError::Field {
            field: name,
            expected: "an integer",
        });
    }
    wrong_type(name, "an integer")
}

pub(super) fn boolean(json: &Json, name: &'static str) -> Result<Option<bool>, SearchError> {
    match json {
        Json::Null => Ok(None),
        Json::Bool(flag) => Ok(Some(*flag)),
        Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            wrong_type(name, "a boolean")
        }
    }
}

const fn wrong_type<T>(
    name: &'static str,
    expected: &'static str,
) -> Result<Option<T>, SearchError> {
    Err(SearchError::Field {
        field: name,
        expected,
    })
}
