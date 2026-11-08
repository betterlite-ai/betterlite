use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthConfigError;
use betterlite_mcp::oauth_resource::python_url::UrlParseError;
use betterlite_mcp::oauth_resource::resource_identity::UrlIdentityError;
use betterlite_mcp::request_headers::raw_headers::{DuplicateAuthorization, RawHeaders};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::helpers::HelperError;

macro_rules! helper_error {
    ($error:ty, $class:literal, $status:expr) => {
        impl From<$error> for Box<HelperError> {
            fn from(error: $error) -> Self {
                invalid($class, $status, error.to_string())
            }
        }
    };
}

helper_error!(UrlParseError, "ValueError", None);
helper_error!(UrlIdentityError, "ValueError", None);
helper_error!(DuplicateAuthorization, "HTTPException", Some(400));
helper_error!(
    TokenEndpointAuthConfigError,
    "TokenEndpointAuthConfigError",
    None
);

pub(super) fn invalid(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn invalid_operation(name: &str) -> Box<HelperError> {
    invalid(
        "UnknownOperation",
        None,
        format!("unknown b4654 function: {name}"),
    )
}

pub(super) fn string<'input>(
    input: &'input Value,
    name: &str,
) -> Result<&'input str, Box<HelperError>> {
    input
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("InvalidInput", None, format!("{name} must be a string")))
}

pub(super) fn optional<'input>(input: &'input Value, name: &str) -> Option<&'input str> {
    if let Some(value) = input.get(name) {
        return value.as_str();
    }
    None
}

pub(super) fn boolean(input: &Value, name: &str) -> Result<bool, Box<HelperError>> {
    input
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("InvalidInput", None, format!("{name} must be a boolean")))
}

pub(super) fn strings<'input>(input: &'input Value, name: &str) -> Option<Vec<&'input str>> {
    let items = input.get(name)?.as_array()?;
    let mut strings = Vec::with_capacity(items.len());
    for item in items {
        if let Some(text) = item.as_str() {
            strings.push(text);
        }
    }
    Some(strings)
}

pub(super) fn pairs(input: &Value) -> Result<Vec<(String, String)>, Box<HelperError>> {
    let raw = input
        .get("raw")
        .cloned()
        .ok_or_else(|| invalid("InvalidInput", None, "raw must be a list".to_owned()))?;
    serde_json::from_value(raw).map_err(|error| invalid("InvalidInput", None, error.to_string()))
}

pub(super) fn parse_headers(raw: &[(String, String)]) -> Result<RawHeaders, Box<HelperError>> {
    let mut pairs: Vec<(&[u8], &[u8])> = Vec::with_capacity(raw.len());
    for (name, value) in raw {
        pairs.push((name.as_bytes(), value.as_bytes()));
    }
    Ok(RawHeaders::from_pairs(&pairs)?)
}

pub(super) fn expose(map: BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    let mut exposed = BTreeMap::new();
    for (name, value) in map {
        exposed.insert(name, value.expose_secret().to_owned());
    }
    exposed
}
