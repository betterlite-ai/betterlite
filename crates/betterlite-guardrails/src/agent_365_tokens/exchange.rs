use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::form_urlencoded;

use crate::agent_365_tokens::Agent365Guardrail;
use crate::agent_365_tokens::cache::{CachedOboToken, assertion_cache_key, parse_expires_in};
use crate::agent_365_tokens::error::{
    Agent365TokenError, Agent365TokenExchangeError, MALFORMED_BAD_ACCESS_TOKEN,
    MALFORMED_MISSING_ACCESS_TOKEN, MALFORMED_NON_JSON_BODY, MALFORMED_NON_OBJECT_BODY,
};
use crate::agent_365_tokens::poster::PostReply;

pub const TOKEN_ENDPOINT_TEMPLATE: &str =
    "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token";
pub const OBO_SCOPE: &str = "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1/ThreatProtection.Evaluate.All";
pub const GRANT_TYPE_JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
pub const REQUESTED_TOKEN_USE: &str = "on_behalf_of";
pub const DEFAULT_TOKEN_ERROR_CODE: &str = "invalid_grant";
pub const MAX_DESCRIPTION_CHARS: usize = 512;

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

struct TokenGrant {
    access_token: String,
    expires_in: f64,
}

#[must_use]
pub fn entra_assertion(value: &GuardrailValue) -> Option<&str> {
    match value {
        GuardrailValue::Text(assertion) if assertion.matches('.').count() == 2 => Some(assertion),
        _ => None,
    }
}

impl Agent365Guardrail {
    pub async fn get_obo_token(&self, assertion: &str) -> Result<SecretString, Agent365TokenError> {
        let cache_key = assertion_cache_key(assertion);
        let cached = {
            let now = (self.clock)();
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.lookup(&cache_key, now)
        };
        if let Some(token) = cached {
            return Ok(token);
        }
        let reply = self.exchange(assertion).await?;
        let grant = parse_token_reply(&reply)?;
        // float_arithmetic is denied; a fused multiply-add adds the grant lifetime to the current time
        let expires_at = grant.expires_in.mul_add(1.0_f64, (self.clock)());
        let token = SecretString::from(grant.access_token);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.insert(
            cache_key,
            CachedOboToken {
                access_token: token.clone(),
                expires_at,
            },
        );
        Ok(token)
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
        let cache_key = assertion_cache_key(assertion);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.evict(&cache_key);
    }

    async fn exchange(&self, assertion: &str) -> Result<PostReply, Agent365TokenError> {
        let url = TOKEN_ENDPOINT_TEMPLATE.replace("{tenant_id}", &self.settings.tenant_id);
        let body = self.form_body(assertion);
        self.post_allowing_error_status(&url, &form_headers(), body.as_bytes())
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

fn form_headers() -> HeaderMap {
    HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static(FORM_CONTENT_TYPE))])
}

fn parse_token_reply(reply: &PostReply) -> Result<TokenGrant, Agent365TokenError> {
    let status = reply.status;
    if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(Agent365TokenError::Throttled {
            status: status.as_u16(),
        });
    }
    if status.is_server_error() {
        return Err(Agent365TokenError::HttpStatus {
            status: status.as_u16(),
        });
    }
    let fields = json_object(&reply.body)?;
    if status.as_u16() >= 400 {
        return Err(Agent365TokenError::Exchange(exchange_failure(
            status, &fields,
        )));
    }
    access_token_grant(&fields)
}

fn json_object(body: &[u8]) -> Result<BTreeMap<String, GuardrailValue>, Agent365TokenError> {
    match serde_json::from_slice::<GuardrailValue>(body) {
        Ok(GuardrailValue::Object(fields)) => Ok(fields),
        Ok(_) => Err(Agent365TokenError::Malformed {
            message: MALFORMED_NON_OBJECT_BODY,
        }),
        Err(_) => Err(Agent365TokenError::Malformed {
            message: MALFORMED_NON_JSON_BODY,
        }),
    }
}

fn exchange_failure(
    status: StatusCode,
    fields: &BTreeMap<String, GuardrailValue>,
) -> Agent365TokenExchangeError {
    let description = optional_text(fields, "error_description").unwrap_or_default();
    Agent365TokenExchangeError {
        status_code: status.as_u16(),
        error_code: optional_text(fields, "error")
            .unwrap_or_else(|| DEFAULT_TOKEN_ERROR_CODE.to_owned()),
        description: description.chars().take(MAX_DESCRIPTION_CHARS).collect(),
        aadsts_codes: parse_aadsts_codes(fields.get("error_codes")),
    }
}

fn access_token_grant(
    fields: &BTreeMap<String, GuardrailValue>,
) -> Result<TokenGrant, Agent365TokenError> {
    let Some(raw) = fields.get("access_token") else {
        return Err(Agent365TokenError::Malformed {
            message: MALFORMED_MISSING_ACCESS_TOKEN,
        });
    };
    let GuardrailValue::Text(access_token) = raw else {
        return Err(Agent365TokenError::Malformed {
            message: MALFORMED_BAD_ACCESS_TOKEN,
        });
    };
    if access_token.is_empty() {
        return Err(Agent365TokenError::Malformed {
            message: MALFORMED_BAD_ACCESS_TOKEN,
        });
    }
    Ok(TokenGrant {
        access_token: access_token.clone(),
        expires_in: parse_expires_in(fields.get("expires_in")),
    })
}

fn parse_aadsts_codes(raw: Option<&GuardrailValue>) -> Vec<i64> {
    match raw {
        Some(GuardrailValue::Array(codes)) => {
            let parsed: Option<Vec<i64>> = codes.iter().map(code_as_i64).collect();
            parsed.unwrap_or_default()
        }
        Some(_) | None => Vec::new(),
    }
}

// serde's untagged GuardrailValue tries SignedInteger first, so every JSON integer that fits i64 arrives signed
fn code_as_i64(value: &GuardrailValue) -> Option<i64> {
    match value {
        GuardrailValue::SignedInteger(code) => Some(*code),
        _ => None,
    }
}

fn optional_text(fields: &BTreeMap<String, GuardrailValue>, key: &str) -> Option<String> {
    match fields.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}
