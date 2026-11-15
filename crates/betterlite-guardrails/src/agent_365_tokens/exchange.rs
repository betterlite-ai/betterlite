use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use secrecy::ExposeSecret as _;
use url::form_urlencoded;

use crate::agent_365_tokens::Agent365Settings;
use crate::agent_365_tokens::error::Agent365TokenError;
use crate::agent_365_tokens::poster::PostReply;
use crate::agent_365_tokens::wire::{TokenGrant, decode_token_reply};

pub const TOKEN_ENDPOINT_TEMPLATE: &str =
    "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token";
pub const OBO_SCOPE: &str = "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1/ThreatProtection.Evaluate.All";
pub const GRANT_TYPE_JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
pub const REQUESTED_TOKEN_USE: &str = "on_behalf_of";

pub const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

#[must_use]
pub fn entra_assertion(value: &GuardrailValue) -> Option<&str> {
    let GuardrailValue::Text(assertion) = value else {
        return None;
    };
    (assertion.matches('.').count() == 2).then_some(assertion.as_str())
}

#[must_use]
pub fn token_form(settings: &Agent365Settings, assertion: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", GRANT_TYPE_JWT_BEARER)
        .append_pair("client_id", &settings.client_id)
        .append_pair("client_secret", settings.client_secret.expose_secret())
        .append_pair("assertion", assertion)
        .append_pair("scope", OBO_SCOPE)
        .append_pair("requested_token_use", REQUESTED_TOKEN_USE)
        .finish()
}

pub fn parse_token_reply(reply: &PostReply) -> Result<TokenGrant, Agent365TokenError> {
    let status = reply.status;
    let code = status.as_u16();
    if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Agent365TokenError::Throttled { status: code });
    }
    if status.is_server_error() {
        return Err(Agent365TokenError::HttpStatus { status: code });
    }
    let body = decode_token_reply(&reply.body)?;
    if code >= 400 {
        return Err(Agent365TokenError::Exchange(body.exchange_failure(code)));
    }
    body.into_grant()
}
