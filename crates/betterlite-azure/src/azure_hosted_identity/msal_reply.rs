use secrecy::SecretString;

use crate::azure_client_secret::token::AccessToken;

use super::decode::{Expiry, HostedTokenReply, ReplyScalar};
use super::environment::{HostedEnvironment, REDACTED};
use super::error::HostedIdentityError;

pub(crate) const REFRESH_THRESHOLD_SECONDS: i64 = 7200;
pub(crate) const DEFAULT_TOKEN_TYPE: &str = "Bearer";

#[derive(Debug, Clone, Copy)]
pub enum MsalKind {
    AppService,
    AzureMl,
    ServiceFabric,
}

#[derive(Debug)]
pub enum MsalReply {
    Token {
        access_token: SecretString,
        expires_in: i64,
        token_type: String,
    },
    Error {
        error: String,
        description: String,
    },
}

fn scalar_text(value: Option<&ReplyScalar>) -> String {
    match value {
        None => "None".to_owned(),
        Some(ReplyScalar::Text(text)) => text.clone(),
        Some(ReplyScalar::Number(number)) => number.to_string(),
        Some(ReplyScalar::Bool(true)) => "True".to_owned(),
        Some(ReplyScalar::Bool(false)) => "False".to_owned(),
    }
}

pub fn msal_reply(
    kind: MsalKind,
    reply: &HostedTokenReply,
    environment: &HostedEnvironment,
    now: i64,
    raw: &str,
) -> Result<MsalReply, HostedIdentityError> {
    if let Some(token) = reply.token()
        && let Some(deadline) = Expiry::nonzero_seconds(reply.expires_on.as_ref(), "expires_on")?
    {
        let token_type = match (kind, reply.token_type.clone()) {
            (MsalKind::ServiceFabric, None) => {
                return Err(HostedIdentityError::MissingField {
                    field: "token_type",
                });
            }
            (_, token_type) => token_type.unwrap_or_else(|| DEFAULT_TOKEN_TYPE.to_owned()),
        };
        return Ok(MsalReply::Token {
            access_token: SecretString::from(token),
            expires_in: deadline
                .checked_sub(now)
                .ok_or(HostedIdentityError::ExpiryOverflow)?,
            token_type,
        });
    }
    let (error, description) = match kind {
        MsalKind::AppService => (
            "invalid_scope",
            format!(
                "{}, {}",
                scalar_text(reply.status_code.as_ref()),
                scalar_text(reply.message.as_ref())
            ),
        ),
        MsalKind::AzureMl => ("invalid_scope", raw.to_owned()),
        MsalKind::ServiceFabric => (service_fabric_error(reply), raw.to_owned()),
    };
    let redacted = environment.redact(&description);
    Ok(MsalReply::Error {
        error: error.to_owned(),
        description: reply.token().map_or_else(
            || redacted.clone(),
            |token| redacted.replace(token, REDACTED),
        ),
    })
}

fn service_fabric_error(reply: &HostedTokenReply) -> &'static str {
    let code = reply
        .error
        .as_ref()
        .and_then(|failure| failure.code.as_ref());
    match code {
        Some(ReplyScalar::Text(text)) if text == "SecretHeaderNotFound" => "unauthorized_client",
        Some(ReplyScalar::Text(text)) if text == "ManagedIdentityNotFound" => "invalid_client",
        Some(ReplyScalar::Text(text)) if text == "ArgumentNullOrEmpty" => "invalid_scope",
        _ => "invalid_request",
    }
}

pub fn complete_msal(
    reply: MsalReply,
    prefix: &'static str,
    started: i64,
    finished: i64,
) -> Result<AccessToken, HostedIdentityError> {
    match reply {
        MsalReply::Error { error, description } => Err(HostedIdentityError::TokenRequest {
            prefix,
            error,
            description,
        }),
        MsalReply::Token {
            access_token,
            expires_in,
            token_type,
        } => Ok(AccessToken {
            token: access_token,
            expires_on: finished
                .checked_add(expires_in)
                .ok_or(HostedIdentityError::ExpiryOverflow)?,
            token_type,
            refresh_on: (expires_in >= REFRESH_THRESHOLD_SECONDS)
                .then(|| started.checked_add(expires_in.div_euclid(2)))
                .map(|refresh| refresh.ok_or(HostedIdentityError::ExpiryOverflow))
                .transpose()?,
        }),
    }
}
