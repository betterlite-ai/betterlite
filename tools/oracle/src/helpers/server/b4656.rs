mod ops;
mod support;

use std::collections::VecDeque;

use betterlite_server::mcp::outbound_credentials::{OAuthToken, TokenStoreUnavailable};
use chrono::DateTime;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

type StoreResult = Result<Option<OAuthToken>, TokenStoreUnavailable>;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("token") => ops::token(input),
            Some("codec") => ops::codec(input),
            Some("coordinator") => ops::coordinator(input),
            Some("store") => ops::store(input),
            _ => Err(value_error("unknown function".to_owned())),
        }
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

pub(super) fn text<'input>(
    value: &'input Value,
    name: &str,
) -> Result<&'input str, Box<HelperError>> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| value_error(format!("{name} is required")))
}

pub(super) fn number(value: &Value, name: &str) -> Result<i64, Box<HelperError>> {
    value
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| value_error(format!("{name} is required")))
}

pub(super) fn parse_token(value: &Value) -> Result<OAuthToken, Box<HelperError>> {
    let expires_at = match value.get("expires_at_ms").and_then(Value::as_i64) {
        Some(millis) => Some(
            DateTime::from_timestamp_millis(millis)
                .ok_or_else(|| value_error(format!("invalid epoch millis: {millis}")))?,
        ),
        None => None,
    };
    let scopes = value
        .get("scopes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(OAuthToken {
        access_token: text(value, "access_token")?.to_owned(),
        expires_at,
        refresh_token: value
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_owned),
        scopes,
        identity_binding_proof: value
            .get("identity_binding_proof")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub(super) fn outcome(value: &Value) -> Result<StoreResult, Box<HelperError>> {
    match value.get("kind").and_then(Value::as_str) {
        Some("absent") => Ok(Ok(None)),
        Some("unavailable") => Ok(Err(TokenStoreUnavailable {
            detail: "unavailable".to_owned(),
        })),
        _ => Ok(Ok(Some(parse_token(
            value.get("token").unwrap_or(&Value::Null),
        )?))),
    }
}

pub(super) fn outcomes(value: Option<&Value>) -> Result<VecDeque<StoreResult>, Box<HelperError>> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(outcome)
                .collect::<Result<VecDeque<_>, _>>()
        })
        .transpose()
        .map(Option::unwrap_or_default)
        .map_err(|error: Box<HelperError>| error)
}

pub(super) fn view(token: &OAuthToken) -> Value {
    serde_json::json!({
        "access_token": token.access_token,
        "expires_at_ms": token.expires_at.map(|expires_at| expires_at.timestamp_millis()),
        "refresh_token": token.refresh_token,
        "scopes": token.scopes,
        "identity_binding_proof": token.identity_binding_proof,
    })
}

pub(super) fn result_view(result: StoreResult) -> Value {
    match result {
        Ok(Some(token)) => view(&token),
        Ok(None) => Value::Null,
        Err(_) => serde_json::json!({"error": "unavailable"}),
    }
}

pub(super) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
