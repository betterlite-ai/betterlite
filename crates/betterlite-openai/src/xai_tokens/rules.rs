use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use url::Url;

use super::codec::{Numeric, TokenPayload};
use super::error::XaiOauthError;

pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
pub const EXPIRY_SKEW_SECONDS: i32 = 120;
const DEFAULT_LIFETIME_SECONDS: i64 = 3600;

#[derive(Debug, Clone)]
pub struct OauthRecord {
    pub access_token: SecretString,
    pub refresh_token: SecretString,
    pub id_token: Option<SecretString>,
    pub token_type: String,
    pub token_endpoint: String,
    pub expires_at: i64,
}

#[derive(Serialize)]
pub struct StoredRecord<'record> {
    access_token: &'record str,
    refresh_token: &'record str,
    id_token: Option<&'record str>,
    token_type: &'record str,
    token_endpoint: &'record str,
    expires_at: i64,
}

impl OauthRecord {
    #[must_use]
    pub fn stored(&self) -> StoredRecord<'_> {
        StoredRecord {
            access_token: self.access_token.expose_secret(),
            refresh_token: self.refresh_token.expose_secret(),
            id_token: self.id_token.as_ref().map(ExposeSecret::expose_secret),
            token_type: &self.token_type,
            token_endpoint: &self.token_endpoint,
            expires_at: self.expires_at,
        }
    }
}

#[must_use]
pub fn is_expired(now: Decimal, expires_at: Option<&Numeric>) -> bool {
    expires_at
        .and_then(Numeric::decimal)
        .and_then(|at| at.checked_sub(Decimal::from(EXPIRY_SKEW_SECONDS)))
        .is_none_or(|boundary| now >= boundary)
}

pub fn validate_xai_endpoint(url: &str) -> Result<Url, XaiOauthError> {
    let parsed = Url::parse(url).ok().filter(|parsed| {
        parsed.scheme() == "https"
            && parsed
                .host_str()
                .is_some_and(|host| host == "x.ai" || host.ends_with(".x.ai"))
    });
    parsed.ok_or(XaiOauthError::UnexpectedEndpoint)
}

pub fn build_auth_record(
    now: Decimal,
    payload: TokenPayload,
    token_endpoint: &str,
    fallback_refresh_token: Option<&SecretString>,
) -> Result<OauthRecord, XaiOauthError> {
    let access_token = payload
        .access_token
        .filter(|token| !token.expose_secret().is_empty())
        .ok_or(XaiOauthError::MissingAccessToken)?;
    let refresh_token = payload
        .refresh_token
        .into_iter()
        .chain(fallback_refresh_token.cloned())
        .find(|token| !token.expose_secret().is_empty())
        .ok_or(XaiOauthError::MissingRefreshToken)?;
    let lifetime = payload
        .expires_in
        .as_ref()
        .and_then(Numeric::seconds)
        .unwrap_or(DEFAULT_LIFETIME_SECONDS);
    let expires_at = now
        .checked_add(Decimal::from(lifetime))
        .and_then(|at| at.trunc().to_i64())
        .unwrap_or(i64::MAX);
    Ok(OauthRecord {
        access_token,
        refresh_token,
        id_token: payload.id_token,
        token_type: payload
            .token_type
            .filter(|kind| !kind.is_empty())
            .unwrap_or_else(|| "Bearer".to_owned()),
        token_endpoint: token_endpoint.to_owned(),
        expires_at,
    })
}
