mod codec;
mod coordinator;
mod ops;
mod support;
mod token;

use std::collections::VecDeque;

use betterlite_server::mcp::outbound_credentials::{OAuthToken, TokenStoreUnavailable};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

type StoreResult = Result<Option<OAuthToken>, TokenStoreUnavailable>;

#[derive(Deserialize)]
struct TokenInput {
    access_token: String,
    #[serde(default)]
    expires_at_ms: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    identity_binding_proof: Option<String>,
}

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("token") => token::token(input),
            Some("codec") => codec::codec(input),
            Some("coordinator") => coordinator::coordinator(input),
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
    let input: TokenInput =
        serde_json::from_value(value.clone()).map_err(|error| value_error(error.to_string()))?;
    let expires_at = match input.expires_at_ms {
        Some(millis) => Some(
            DateTime::from_timestamp_millis(millis)
                .ok_or_else(|| value_error(format!("invalid epoch millis: {millis}")))?,
        ),
        None => None,
    };
    Ok(OAuthToken {
        access_token: input.access_token,
        expires_at,
        refresh_token: input.refresh_token,
        scopes: input.scopes,
        identity_binding_proof: input.identity_binding_proof,
    })
}

pub(super) fn outcome(value: &Value) -> Result<StoreResult, Box<HelperError>> {
    let kind = value.get("kind").and_then(Value::as_str);
    if kind == Some("absent") {
        return Ok(Ok(None));
    }
    if kind == Some("unavailable") {
        return Ok(Err(TokenStoreUnavailable {
            detail: "unavailable".to_owned(),
        }));
    }
    let token = parse_token(value.get("token").unwrap_or(&Value::Null))?;
    Ok(Ok(Some(token)))
}

pub(super) fn outcomes(value: Option<&Value>) -> Result<VecDeque<StoreResult>, Box<HelperError>> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Ok(VecDeque::new());
    };
    items.iter().map(outcome).collect()
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
