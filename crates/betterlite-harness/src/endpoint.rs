pub mod catalog_prices;
pub mod clock;
pub mod cost;
pub mod encode;
pub mod errors;
pub mod headers;
pub mod random;
pub mod redact;
pub mod route;
pub mod sse;
mod tests;
pub mod usage;

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::HeaderMap;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use thiserror::Error;

use crate::context::GatewayTarget;
use crate::types::{Harness, JsonObject, Usage};

use cost::{CostPerToken, compute_cost};
use errors::{ErrorBody, ErrorDetail, ErrorReply, Failure};
use headers::extract_token;
use random::RandomBytes;
use usage::UsageTracker;

pub const HARNESS_SESSION_TOKEN_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum EndpointError {
    #[error("the session token could not be generated")]
    Token {
        #[source]
        source: getrandom::Error,
    },
}

#[derive(Debug)]
pub struct EndpointSetup {
    pub harness: Harness,
    pub model: Option<String>,
    pub gateway: Option<GatewayTarget>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub metadata: JsonObject,
}

pub struct ModelEndpoint {
    pub harness: Harness,
    pub model: Option<String>,
    pub gateway: Option<GatewayTarget>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub metadata: JsonObject,
    pub token: SecretString,
    pub usage: Mutex<UsageTracker>,
    pub prices: Arc<dyn CostPerToken>,
}

impl ModelEndpoint {
    pub fn new(
        setup: EndpointSetup,
        prices: Arc<dyn CostPerToken>,
        random: &dyn RandomBytes,
    ) -> Result<Self, EndpointError> {
        let mut bytes = [0_u8; HARNESS_SESSION_TOKEN_BYTES];
        random
            .fill(&mut bytes)
            .map_err(|source| EndpointError::Token { source })?;
        Ok(Self {
            harness: setup.harness,
            model: setup.model,
            gateway: setup.gateway,
            api_key: setup.api_key,
            api_base: setup.api_base,
            metadata: setup.metadata,
            token: SecretString::from(URL_SAFE_NO_PAD.encode(bytes)),
            usage: Mutex::new(UsageTracker::default()),
            prices,
        })
    }

    #[must_use]
    pub fn authorized(&self, request_headers: &HeaderMap) -> bool {
        extract_token(request_headers).is_some_and(|token| {
            bool::from(
                token
                    .expose_secret()
                    .as_bytes()
                    .ct_eq(self.token.expose_secret().as_bytes()),
            )
        })
    }

    fn secrets(&self) -> [Option<&SecretString>; 3] {
        let gateway_key = self.gateway.as_ref().map(|gateway| &gateway.api_key);
        let api_key = self.api_key.as_ref();
        [gateway_key, api_key, Some(&self.token)]
    }

    #[must_use]
    pub fn unauthorized() -> ErrorReply {
        ErrorReply {
            status: 401,
            body: ErrorBody {
                error: ErrorDetail {
                    error_type: "authentication_error".to_owned(),
                    message: "invalid token".to_owned(),
                },
            },
        }
    }

    #[must_use]
    pub fn error(&self, failure: &Failure, explicit_status: Option<u16>) -> ErrorReply {
        let message = redact::sanitize(&failure.message, &self.secrets());
        let status = match explicit_status {
            Some(code) if code != 0 => code,
            _ => failure
                .status
                .filter(|code| (400..=599).contains(code))
                .unwrap_or(500),
        };
        ErrorReply {
            status,
            body: ErrorBody {
                error: ErrorDetail {
                    error_type: failure.error_type.clone(),
                    message,
                },
            },
        }
    }

    pub fn record(
        &self,
        model: Option<&str>,
        input_tokens: u64,
        output_tokens: u64,
        cost: Option<Decimal>,
    ) {
        let charge = cost.unwrap_or_else(|| {
            compute_cost(self.prices.as_ref(), model, input_tokens, output_tokens)
        });
        let mut usage = match self.usage.lock() {
            Ok(usage) => usage,
            Err(poisoned) => poisoned.into_inner(),
        };
        usage.add(input_tokens, output_tokens, charge);
    }

    #[must_use]
    pub fn usage(&self) -> Usage {
        match self.usage.lock() {
            Ok(usage) => usage.snapshot(),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        }
    }

    #[must_use]
    pub fn cost(&self) -> Decimal {
        match self.usage.lock() {
            Ok(usage) => usage.cost,
            Err(poisoned) => poisoned.into_inner().cost,
        }
    }
}
