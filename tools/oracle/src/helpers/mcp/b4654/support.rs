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

pub(super) fn function_name(input: &Value) -> Result<&str, Box<HelperError>> {
    let Some(function) = input.get("function").and_then(Value::as_str) else {
        return Err(invalid("InvalidInput", None, "no function".to_owned()));
    };
    Ok(function)
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

pub(super) fn optional<'input>(
    input: &'input Value,
    name: &str,
) -> Result<Option<&'input str>, Box<HelperError>> {
    match input.get(name) {
        Some(Value::String(text)) => Ok(Some(text)),
        None | Some(Value::Null) => Ok(None),
        Some(_) => Err(invalid(
            "ValueError",
            None,
            format!("{name} must be a string"),
        )),
    }
}

pub(super) fn boolean(input: &Value, name: &str) -> Result<bool, Box<HelperError>> {
    let Some(value) = input.get(name).and_then(Value::as_bool) else {
        return Err(invalid(
            "InvalidInput",
            None,
            format!("{name} must be a boolean"),
        ));
    };
    Ok(value)
}

pub(super) fn parse_headers(input: &Value) -> Result<RawHeaders, Box<HelperError>> {
    let raw = input
        .get("raw")
        .cloned()
        .ok_or_else(|| invalid("InvalidInput", None, "raw must be a list".to_owned()))?;
    let entries: Vec<(String, String)> = serde_json::from_value(raw)
        .map_err(|error| invalid("InvalidInput", None, error.to_string()))?;
    let mut pairs: Vec<(&[u8], &[u8])> = Vec::with_capacity(entries.len());
    for (name, value) in &entries {
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

pub(super) fn expose_nested(
    map: BTreeMap<String, BTreeMap<String, SecretString>>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    map.into_iter()
        .map(|(alias, headers)| (alias, expose(headers)))
        .collect()
}
