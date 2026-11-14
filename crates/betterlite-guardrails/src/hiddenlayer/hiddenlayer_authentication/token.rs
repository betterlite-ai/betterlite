use std::io::{self, Read as _};
use std::time::Duration;

use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use reqwest::blocking::ClientBuilder;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::headroom::text::lossy_text;
use crate::primitives::json_value::JsonValue;

#[derive(Debug, Clone)]
pub struct TokenRequest {
    pub url: String,
    pub api_id: String,
    pub api_key: SecretString,
    pub timeout: Duration,
}

#[derive(Debug, Error)]
pub enum JwtError {
    #[error("the HiddenLayer token request could not be sent")]
    Transport {
        #[source]
        source: reqwest::Error,
    },
    #[error("Unable to get authentication credentials for the HiddenLayer API: {status}: {body}")]
    Status { status: u16, body: String },
    #[error(
        "Unable to get authentication credentials for the HiddenLayer API - invalid response: {body}"
    )]
    InvalidResponse { body: String },
    #[error("the HiddenLayer token response is not JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the HiddenLayer token response body could not be read")]
    Body {
        #[source]
        source: io::Error,
    },
    #[error("the HiddenLayer token response body is larger than {limit} bytes")]
    TooLarge { limit: usize },
}

// LiteLLM fetches the token with a blocking requests.post inside its constructor
pub trait JwtTransport: Send + Sync {
    fn fetch_token(&self, request: &TokenRequest) -> Result<String, JwtError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReqwestJwtTransport;

impl JwtTransport for ReqwestJwtTransport {
    fn fetch_token(&self, request: &TokenRequest) -> Result<String, JwtError> {
        let client = ClientBuilder::from(base_builder())
            .timeout(request.timeout)
            .build()
            .map_err(|source| JwtError::Transport { source })?;
        let response = client
            .post(&request.url)
            .basic_auth(
                request.api_id.as_str(),
                Some(request.api_key.expose_secret()),
            )
            .send()
            .map_err(|source| JwtError::Transport { source })?;
        let status = response.status().as_u16();
        let limit = TransportLimits::DEFAULT.body_bytes;
        let capped = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
        let mut bytes = Vec::new();
        response
            .take(capped)
            .read_to_end(&mut bytes)
            .map_err(|source| JwtError::Body { source })?;
        if bytes.len() > limit {
            return Err(JwtError::TooLarge { limit });
        }
        let body = lossy_text(&bytes);
        if status >= 400 {
            return Err(JwtError::Status { status, body });
        }
        parse_token(&body)
    }
}

fn parse_token(body: &str) -> Result<String, JwtError> {
    let decoded =
        serde_json::from_str::<JsonValue>(body).map_err(|source| JwtError::Decode { source })?;
    match decoded {
        JsonValue::Object(fields) => match fields.get("access_token") {
            Some(JsonValue::String(token)) => Ok(token.clone()),
            _ => Err(JwtError::InvalidResponse {
                body: body.to_owned(),
            }),
        },
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Err(JwtError::InvalidResponse {
            body: body.to_owned(),
        }),
    }
}
