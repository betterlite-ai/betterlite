use std::num::ParseIntError;
use std::str::Utf8Error;
use std::time::Duration;

use axum::http::header::{InvalidHeaderValue, USER_AGENT};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use secrecy::{ExposeSecret as _, SecretString};
use sha1::{Digest as _, Sha1};
use thiserror::Error;
use url::Url;

use super::error::PasswordError;

pub const HIBP_RANGE_API_BASE: &str = "https://api.pwnedpasswords.com/range";
pub const HIBP_TIMEOUT: Duration = Duration::from_secs(5);
const LITELLM_VERSION: &str = "1.105.1";
const PREFIX_LENGTH: usize = 5;

#[derive(Debug, Error)]
pub enum LookupError {
    #[error("the range URL is invalid")]
    Url(#[source] url::ParseError),
    #[error("the request headers are invalid")]
    Header(#[source] InvalidHeaderValue),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the range API answered with status {status}")]
    Status { status: u16 },
    #[error("the range API body is not UTF-8")]
    Body(#[source] Utf8Error),
    #[error("the match count {count:?} is not a number")]
    Count {
        count: String,
        #[source]
        source: ParseIntError,
    },
}

pub struct BreachChecker {
    pub client: ProxiedClient,
    pub base: String,
    pub timeout: Duration,
    pub enabled: bool,
}

// The characters Python's str.splitlines breaks lines on.
const fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn suffix_listed(body: &str, suffix: &str) -> Result<bool, LookupError> {
    for line in body.to_uppercase().split(is_line_break) {
        let trimmed = line.trim();
        let (entry_suffix, raw_count) = trimmed.split_once(':').unwrap_or((trimmed, ""));
        if entry_suffix != suffix {
            continue;
        }
        let count = raw_count.trim();
        if count.is_empty() {
            return Ok(false);
        }
        let parsed = count.parse::<i128>().map_err(|source| LookupError::Count {
            count: count.to_owned(),
            source,
        })?;
        return Ok(parsed > 0);
    }
    Ok(false)
}

impl BreachChecker {
    // SHA-1 is only the lookup key into the HIBP dataset; just its first five characters leave the process.
    pub async fn lookup(&self, password: &SecretString) -> Result<bool, LookupError> {
        let digest = hex::encode_upper(Sha1::digest(password.expose_secret().as_bytes()));
        let (prefix, suffix) = digest.split_at(PREFIX_LENGTH);
        let url = Url::parse(&format!("{}/{prefix}", self.base)).map_err(LookupError::Url)?;
        let user_agent = HeaderValue::from_str(&format!("litellm-proxy/{LITELLM_VERSION}"))
            .map_err(LookupError::Header)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("add-padding"),
            HeaderValue::from_static("true"),
        );
        headers.insert(USER_AGENT, user_agent);
        let response = self
            .client
            .send_buffered(OutboundRequest {
                method: Method::GET,
                url,
                query: Vec::new(),
                headers,
                body: RequestBody::Empty,
                deadline: self.timeout,
            })
            .await?;
        if !(200..300).contains(&response.status) {
            return Err(LookupError::Status {
                status: response.status,
            });
        }
        let body = str::from_utf8(&response.body).map_err(LookupError::Body)?;
        suffix_listed(body, suffix)
    }

    // Fails open: an unreachable or misbehaving range API allows the password.
    pub async fn is_breached(&self, password: &SecretString) -> bool {
        if !self.enabled {
            return false;
        }
        match self.lookup(password).await {
            Ok(breached) => breached,
            Err(error) => {
                tracing::warn!("Breached-password check skipped, HIBP lookup failed: {error}");
                false
            }
        }
    }

    pub async fn validate_not_breached(
        &self,
        password: &SecretString,
    ) -> Result<(), PasswordError> {
        if self.is_breached(password).await {
            return Err(PasswordError::Breached);
        }
        Ok(())
    }
}
