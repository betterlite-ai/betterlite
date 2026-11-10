use betterlite_redact::secret::Redactor;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_client_secret::token::AccessToken;

use super::decode::{Expiry, HostedTokenReply, ReplyScalar, decode_hosted_identity_token};
use super::environment::{HostedEnvironment, REDACTED};
use super::error::HostedIdentityError;
use super::request::{HostedIo, HostedResponse};

const APP_SERVICE_PREFIX: &str =
    "App Service managed identity configuration not found in environment. ";
const AZURE_ML_PREFIX: &str = "Azure ML managed identity configuration not found in environment. ";
const SERVICE_FABRIC_PREFIX: &str =
    "Service Fabric managed identity configuration not found in environment. ";

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
        Some(ReplyScalar::Nested(_)) => "<nested>".to_owned(),
    }
}

pub fn msal_response(
    kind: MsalKind,
    response: &HostedResponse,
    environment: &HostedEnvironment,
    started: i64,
    io: &HostedIo<'_>,
) -> Result<AccessToken, HostedIdentityError> {
    let response_time = io.clock.now_unix_seconds();
    let raw = response.body.expose_secret();
    let reply = decode_hosted_identity_token(raw.as_bytes())?.ok_or_else(|| {
        HostedIdentityError::ReplyValue {
            field: "$".to_owned(),
        }
    })?;
    let scrub = Scrub {
        environment,
        redactor: io.redactor,
    };
    let normalized = msal_reply(kind, &reply, &scrub, response_time, raw)?;
    let prefix = match kind {
        MsalKind::AppService => APP_SERVICE_PREFIX,
        MsalKind::AzureMl => AZURE_ML_PREFIX,
        MsalKind::ServiceFabric => SERVICE_FABRIC_PREFIX,
    };
    // the SDK reads its own clock after MSAL normalized the reply, so a slow request shifts expires_on
    complete_msal(normalized, prefix, started, io.clock.now_unix_seconds())
}

struct Scrub<'scrub> {
    environment: &'scrub HostedEnvironment,
    redactor: &'scrub Redactor,
}

impl Scrub<'_> {
    // Redactor masks key and prefix shaped secrets; the opaque identity header and the returned token need their exact values replaced
    fn description(&self, text: &str, token: Option<&SecretString>) -> String {
        let redacted = self
            .environment
            .redact_known(&self.redactor.redact_string(text));
        match token {
            Some(secret) => redacted.replace(secret.expose_secret(), REDACTED),
            None => redacted,
        }
    }
}

fn msal_reply(
    kind: MsalKind,
    reply: &HostedTokenReply,
    scrub: &Scrub<'_>,
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
            access_token: token.clone(),
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
        MsalKind::ServiceFabric => (service_fabric_error(reply)?, raw.to_owned()),
    };
    Ok(MsalReply::Error {
        error: error.to_owned(),
        description: scrub.description(&description, reply.token()),
    })
}

fn service_fabric_error(reply: &HostedTokenReply) -> Result<&'static str, HostedIdentityError> {
    let failure = reply
        .error
        .as_ref()
        .map(|member| member.read("error"))
        .transpose()?;
    Ok(match failure.and_then(|detail| detail.code.as_ref()) {
        Some(ReplyScalar::Text(text)) if text == "SecretHeaderNotFound" => "unauthorized_client",
        Some(ReplyScalar::Text(text)) if text == "ManagedIdentityNotFound" => "invalid_client",
        Some(ReplyScalar::Text(text)) if text == "ArgumentNullOrEmpty" => "invalid_scope",
        _ => "invalid_request",
    })
}

fn complete_msal(
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
