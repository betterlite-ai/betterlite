use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, Method};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use url::Url;

use super::error::WorkloadIdentityError;

pub const TOKEN_EXCHANGE_URL: &str = "https://auth.openai.com/oauth/token";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:jwt";
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);
const OAUTH_FALLBACK_MESSAGE: &str = "OAuth authentication error.";

#[derive(Serialize)]
struct ExchangeBody<'text> {
    grant_type: &'static str,
    subject_token: &'text str,
    subject_token_type: &'static str,
    identity_provider_id: &'text str,
    service_account_id: &'text str,
}

// a field of the wrong JSON type reads as absent, the way the SDK's isinstance checks do
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Value(T),
    Other(IgnoredAny),
}

impl<T> Lenient<T> {
    fn into_value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Other(_ignored) => None,
        }
    }
}

#[derive(Deserialize)]
struct RawExchange {
    access_token: Option<Lenient<String>>,
    // Number, not f64: arbitrary_precision buffers a JSON number as a private map, which only Number reads back
    expires_in: Option<Lenient<serde_json::Number>>,
}

#[derive(Default, Deserialize)]
struct RawOAuthError {
    error: Option<Lenient<String>>,
    error_description: Option<Lenient<String>>,
}

#[derive(Debug)]
pub struct TokenExchange {
    pub access_token: SecretString,
    pub expires_in: f64,
}

pub fn exchange_request(
    exchange_url: &str,
    subject_token: &SecretString,
    identity_provider_id: &str,
    service_account_id: &str,
) -> Result<OutboundRequest, WorkloadIdentityError> {
    let body = ExchangeBody {
        grant_type: GRANT_TYPE,
        subject_token: subject_token.expose_secret(),
        subject_token_type: SUBJECT_TOKEN_TYPE,
        identity_provider_id,
        service_account_id,
    };
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(exchange_url).map_err(WorkloadIdentityError::ExchangeUrl)?,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(serde_json::to_vec(&body).map_err(WorkloadIdentityError::Encode)?),
        deadline: EXCHANGE_DEADLINE,
    })
}

pub fn decode_token_exchange(bytes: &[u8]) -> Result<TokenExchange, WorkloadIdentityError> {
    let raw = match serde_json::from_slice::<Option<RawExchange>>(bytes) {
        Ok(Some(raw)) => raw,
        Ok(None) => return Err(WorkloadIdentityError::EmptyBody),
        Err(error) if error.is_data() => {
            return Err(WorkloadIdentityError::InvalidField {
                field: "access_token",
            });
        }
        Err(_syntax) => return Err(WorkloadIdentityError::EmptyBody),
    };
    let access_token = raw
        .access_token
        .and_then(Lenient::into_value)
        .filter(|text| !text.is_empty())
        .ok_or(WorkloadIdentityError::InvalidField {
            field: "access_token",
        })?;
    let expires_in = raw
        .expires_in
        .and_then(Lenient::into_value)
        .as_ref()
        .and_then(serde_json::Number::as_f64)
        .ok_or(WorkloadIdentityError::InvalidField {
            field: "expires_in",
        })?;
    Ok(TokenExchange {
        access_token: SecretString::from(access_token),
        expires_in,
    })
}

pub fn token_from_response(
    response: &UpstreamResponse,
) -> Result<TokenExchange, WorkloadIdentityError> {
    let status = response.status;
    if matches!(status, 400 | 401 | 403) {
        let body = serde_json::from_slice::<Option<RawOAuthError>>(&response.body)
            .ok()
            .flatten()
            .unwrap_or_default();
        let message = body
            .error_description
            .and_then(Lenient::into_value)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| OAUTH_FALLBACK_MESSAGE.to_owned());
        return Err(WorkloadIdentityError::OAuth {
            status,
            error: body.error.and_then(Lenient::into_value),
            message,
        });
    }
    if (200..300).contains(&status) {
        return decode_token_exchange(&response.body);
    }
    Err(WorkloadIdentityError::ExchangeStatus { status })
}
