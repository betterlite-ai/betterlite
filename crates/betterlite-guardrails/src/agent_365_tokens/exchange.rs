use std::sync::PoisonError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use url::form_urlencoded;

use crate::agent_365_tokens::Agent365Guardrail;
use crate::agent_365_tokens::cache::{CachedOboToken, TOKEN_EXPIRY_MARGIN};
use crate::agent_365_tokens::error::Agent365TokenError;
use crate::agent_365_tokens::poster::PostReply;
use crate::agent_365_tokens::wire::{TokenGrant, decode_token_reply};

pub const TOKEN_ENDPOINT_TEMPLATE: &str =
    "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token";
pub const OBO_SCOPE: &str = "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1/ThreatProtection.Evaluate.All";
pub const GRANT_TYPE_JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
pub const REQUESTED_TOKEN_USE: &str = "on_behalf_of";

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

#[must_use]
pub fn entra_assertion(value: &GuardrailValue) -> Option<&str> {
    match value {
        GuardrailValue::Text(assertion) if assertion.matches('.').count() == 2 => Some(assertion),
        _ => None,
    }
}

impl Agent365Guardrail {
    pub async fn get_obo_token(&self, assertion: &str) -> Result<SecretString, Agent365TokenError> {
        let cache_key = hex::encode(Sha256::digest(assertion.as_bytes()));
        let threshold = (self.clock)().checked_add(TOKEN_EXPIRY_MARGIN);
        let cached = {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            match cache
                .peek(&cache_key)
                .map(|entry| (entry.access_token.clone(), entry.expires_at))
            {
                Some((token, expires_at)) if threshold.is_some_and(|limit| expires_at > limit) => {
                    let _promoted = cache.promote(&cache_key);
                    Some(token)
                }
                _ => None,
            }
        };
        if let Some(token) = cached {
            return Ok(token);
        }
        let grant = parse_token_reply(&self.exchange(assertion).await?)?;
        let Some(expires_at) = (self.clock)().checked_add(grant.expires_in) else {
            return Err(Agent365TokenError::TokenExpiry);
        };
        let entry = CachedOboToken {
            access_token: grant.access_token.clone(),
            expires_at,
        };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let _evicted = cache.push(cache_key, entry);
        Ok(grant.access_token)
    }

    pub async fn post_allowing_error_status(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<PostReply, Agent365TokenError> {
        self.poster
            .post(url, headers, body, self.request_timeout)
            .await
            .map_err(Agent365TokenError::Poster)
    }

    pub fn evict_obo_token(&self, assertion: &str) {
        let cache_key = hex::encode(Sha256::digest(assertion.as_bytes()));
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let _removed = cache.pop(&cache_key);
    }

    async fn exchange(&self, assertion: &str) -> Result<PostReply, Agent365TokenError> {
        let url = TOKEN_ENDPOINT_TEMPLATE.replace("{tenant_id}", &self.settings.tenant_id);
        let body = self.form_body(assertion);
        let headers =
            HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static(FORM_CONTENT_TYPE))]);
        self.post_allowing_error_status(&url, &headers, body.as_bytes())
            .await
    }

    fn form_body(&self, assertion: &str) -> String {
        form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", GRANT_TYPE_JWT_BEARER)
            .append_pair("client_id", &self.settings.client_id)
            .append_pair("client_secret", self.settings.client_secret.expose_secret())
            .append_pair("assertion", assertion)
            .append_pair("scope", OBO_SCOPE)
            .append_pair("requested_token_use", REQUESTED_TOKEN_USE)
            .finish()
    }
}

fn parse_token_reply(reply: &PostReply) -> Result<TokenGrant, Agent365TokenError> {
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
