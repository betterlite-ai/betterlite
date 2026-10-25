use betterlite_mcp::session_tokens::{
    is_session_refresh_token, is_session_token, mint_session_refresh_token, mint_session_token,
    open_session_refresh_token, open_session_token,
};
use betterlite_mcp::types::{SessionKeys, SessionPrincipal, SessionSigningKeys};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
#[derive(Deserialize)]
pub(crate) struct Input {
    action: String,
    candidate: Option<String>,
    client_id: Option<String>,
    key: Option<String>,
    now: Option<i64>,
    resource_server_id: Option<String>,
    audience: Option<String>,
    team_id: Option<String>,
    refresh: Option<bool>,
    user_id: Option<String>,
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input.action.as_str() {
            "prefix" => serde_json::json!({
                "session": input.candidate.as_deref().is_some_and(is_session_token),
                "refresh": input.candidate.as_deref().is_some_and(is_session_refresh_token),
            }),
            "mint" => token(input, TokenAction::Mint)?,
            "open" => token(input, TokenAction::Open)?,
            _ => return Ok(serde_json::json!({"error": "unknown session token action"})),
        };
        Ok(output)
    }
}
fn keys(input: &Input) -> SessionSigningKeys {
    let signing_key = SecretString::from(input.key.clone().unwrap_or_default());
    SessionSigningKeys::Symmetric(SessionKeys { signing_key })
}
fn clock(input: &Input) -> Result<DateTime<Utc>, Box<crate::helpers::HelperError>> {
    let timestamp = input.now.unwrap_or_default();
    let clock = DateTime::from_timestamp(timestamp, 0);
    clock.ok_or_else(|| helper_error("invalid now"))
}
fn helper_error(message: &str) -> Box<crate::helpers::HelperError> {
    let owned_message = message.to_owned();
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: owned_message,
    })
}
fn principal(input: &Input) -> SessionPrincipal {
    let client_id = match input.client_id.as_deref() {
        Some("__oversized__") => "x".repeat(4096),
        Some(value) => value.to_owned(),
        None => String::new(),
    };
    SessionPrincipal {
        user_id: input.user_id.clone().unwrap_or_default(),
        client_id,
        resource_server_id: input.resource_server_id.clone(),
        audience: input.audience.clone(),
        team_id: input.team_id.clone(),
    }
}
#[derive(Clone, Copy)]
enum TokenAction {
    Mint,
    Open,
}

fn token(
    input: &Input,
    action: TokenAction,
) -> Result<serde_json::Value, Box<crate::helpers::HelperError>> {
    let now = clock(input)?;
    if matches!(action, TokenAction::Mint) {
        let result = if input.refresh.unwrap_or(false) {
            mint_session_refresh_token(&principal(input), &keys(input), now)
        } else {
            mint_session_token(&principal(input), &keys(input), now)
        };
        return match result {
            Ok(minted) => Ok(serde_json::json!({
                "expires_at": minted.expires_at.timestamp(),
                "prefix": if input.refresh.unwrap_or(false) { "llm_srefresh_" } else { "llm_session_" },
                "token_bytes": minted.token.expose_secret().len(),
            })),
            Err(error) => Ok(serde_json::json!({"error": error.to_string()})),
        };
    }
    let candidate = match input.candidate.as_deref() {
        Some("__oversized__") => format!("llm_session_{}", "x".repeat(4096)),
        Some(value) => value.to_owned(),
        None => String::new(),
    };
    let result = if input.refresh.unwrap_or(false) {
        open_session_refresh_token(&candidate, &keys(input), now)
    } else {
        open_session_token(&candidate, &keys(input), now)
    };
    match result {
        Ok(opened) => Ok(serde_json::json!({
            "client_id": opened.principal.client_id, "exp": opened.exp, "iat": opened.iat,
            "jti": opened.jti,
            "kind": if input.refresh.unwrap_or(false) { "session_refresh" } else { "session" },
            "user_id": opened.principal.user_id,
        })),
        Err(error) => Ok(serde_json::json!({"error": error.to_string()})),
    }
}
