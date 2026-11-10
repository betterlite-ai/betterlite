use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::token::AccessToken;

use super::decode::{Expiry, HostedTokenReply, decode_hosted_identity_token};
use super::environment::HostedEnvironment;
use super::error::HostedIdentityError;
use super::msal_reply::{DEFAULT_TOKEN_TYPE, REFRESH_THRESHOLD_SECONDS};
use super::request::HostedResponse;

const TOKEN_MASK: &str = "****";

pub fn cloud_shell_response(
    response: &HostedResponse,
    environment: &HostedEnvironment,
    request_time: i64,
) -> Result<AccessToken, HostedIdentityError> {
    let raw = response.body.expose_secret();
    if raw.is_empty() {
        return Err(HostedIdentityError::NoToken);
    }
    let reply = match decode_hosted_identity_token(raw.as_bytes()) {
        Ok(reply) => reply,
        Err(HostedIdentityError::ReplyJson(source)) => {
            let content_type = response.content_type.as_deref().unwrap_or_default();
            if content_type.starts_with("application/json") {
                return Err(HostedIdentityError::JsonContentType(source));
            }
            return Err(HostedIdentityError::UnsupportedContentType {
                content_type: content_type.to_owned(),
                source,
            });
        }
        Err(error) => return Err(error),
    };
    let Some(parsed) = reply.as_ref().filter(|candidate| !candidate.is_empty()) else {
        return Err(HostedIdentityError::NoToken);
    };
    let Some(token) = parsed.access_token.as_deref() else {
        return Err(unexpected(parsed, environment, raw));
    };
    if parsed.expires_in.is_none() && parsed.expires_on.is_none() {
        return Err(unexpected(parsed, environment, raw));
    }
    let (expires_on, refresh_on) = cloud_shell_times(parsed, request_time)?;
    if parsed.resource.is_none() {
        return Err(HostedIdentityError::MissingField { field: "resource" });
    }
    Ok(AccessToken {
        token: SecretString::from(token),
        expires_on,
        token_type: parsed
            .token_type
            .clone()
            .unwrap_or_else(|| DEFAULT_TOKEN_TYPE.to_owned()),
        refresh_on,
    })
}

fn cloud_shell_times(
    parsed: &HostedTokenReply,
    request_time: i64,
) -> Result<(i64, Option<i64>), HostedIdentityError> {
    let overflow = || HostedIdentityError::ExpiryOverflow;
    let expires_on = match Expiry::nonzero_seconds(parsed.expires_on.as_ref(), "expires_on")? {
        Some(deadline) => deadline,
        None => parsed
            .expires_in
            .as_ref()
            .ok_or(HostedIdentityError::MissingField {
                field: "expires_in",
            })?
            .seconds("expires_in")?
            .checked_add(request_time)
            .ok_or_else(overflow)?,
    };
    let lifetime = match Expiry::nonzero_seconds(parsed.expires_in.as_ref(), "expires_in")? {
        Some(seconds) => seconds,
        None => expires_on.checked_sub(request_time).ok_or_else(overflow)?,
    };
    let offset = match parsed.refresh_in.as_ref() {
        Some(explicit) => Some(explicit.seconds("refresh_in")?),
        None if lifetime >= REFRESH_THRESHOLD_SECONDS => Some(lifetime.div_euclid(2)),
        None => None,
    };
    let refresh_on = offset
        .map(|seconds| request_time.checked_add(seconds).ok_or_else(overflow))
        .transpose()?;
    Ok((expires_on, refresh_on))
}

fn unexpected(
    reply: &HostedTokenReply,
    environment: &HostedEnvironment,
    raw: &str,
) -> HostedIdentityError {
    let masked = reply.token().map_or_else(
        || raw.to_owned(),
        |token| {
            raw.replace(
                &serde_json::to_string(token).unwrap_or_default(),
                &format!("\"{TOKEN_MASK}\""),
            )
        },
    );
    HostedIdentityError::UnexpectedResponse {
        content: environment.redact(&masked),
    }
}
