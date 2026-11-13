pub mod api_base;
pub mod content;
pub mod error;
mod failure;
mod payload;
pub mod query;
pub mod response;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use error::CompresrTransportError;
use failure::{Failure, REQUEST_FAILED, SERVICE_ERROR};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::generic_guardrail_contracts::UnreachableFallback;

mod tests;

// LiteLLM bounds its shared 600s client with its own compress timeout
pub const DEFAULT_COMPRESS_TIMEOUT: Duration = Duration::from_secs(60);

// Request-content fields a passthrough compression_params key must never replace
const RESERVED_COMPRESSION_PARAM_KEYS: [&str; 3] = ["context", "query", "inputs"];
const SAFE_RESPONSE_TEXT_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressTarget {
    pub context: String,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionResult {
    pub compressed_context: Option<String>,
    pub original_tokens: i64,
    pub compressed_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompressOutcome {
    Results(Vec<CompressionResult>),
    Uncompressed,
}

#[derive(Debug, Clone)]
pub struct TransportSettings {
    pub api_base: String,
    pub model: String,
    pub target_compression_ratio: f64,
    pub coarse: bool,
    pub dynamic: bool,
    pub dynamic_min_ratio: Option<f64>,
    pub dynamic_max_ratio: Option<f64>,
    pub compression_params: BTreeMap<String, GuardrailValue>,
    pub timeout: Duration,
    pub fallback: UnreachableFallback,
}

#[derive(Debug, Clone)]
pub struct CompresrTransport {
    pub settings: TransportSettings,
    pub api_key: SecretString,
    pub client: reqwest::Client,
}

impl CompresrTransport {
    pub fn new(
        settings: TransportSettings,
        api_key: SecretString,
        client: reqwest::Client,
    ) -> Result<Self, CompresrTransportError> {
        let api_base = api_base::validate_api_base(settings.api_base.trim_end_matches('/'))?;
        Ok(Self {
            settings: TransportSettings {
                api_base,
                model: settings.model,
                target_compression_ratio: settings.target_compression_ratio,
                coarse: settings.coarse,
                dynamic: settings.dynamic,
                dynamic_min_ratio: settings.dynamic_min_ratio,
                dynamic_max_ratio: settings.dynamic_max_ratio,
                compression_params: without_reserved_params(settings.compression_params),
                timeout: settings.timeout,
                fallback: settings.fallback,
            },
            api_key,
            client,
        })
    }

    pub async fn compress(
        &self,
        targets: &[CompressTarget],
    ) -> Result<CompressOutcome, CompresrTransportError> {
        let request = payload::build(&self.settings, targets);
        let headers = match self.request_headers() {
            Ok(headers) => headers,
            Err(failure) => return self.failed(failure),
        };
        let body = match serde_json::to_vec(&request.payload) {
            Ok(body) => body,
            Err(source) => {
                return self.failed(Failure {
                    message: REQUEST_FAILED,
                    detail: source.to_string(),
                });
            }
        };
        let sent = self
            .client
            .post(&request.url)
            .headers(headers)
            .body(body)
            .timeout(self.settings.timeout)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(source) => {
                return self.failed(Failure {
                    message: REQUEST_FAILED,
                    detail: source.to_string(),
                });
            }
        };
        let status = response.status();
        let text = response::read_body(response).await;
        if !status.is_success() {
            let detail = format!(
                "status {status}, body {}",
                response::safe_response_text(text.as_deref(), SAFE_RESPONSE_TEXT_LIMIT)
            );
            return self.failed(Failure {
                message: SERVICE_ERROR,
                detail,
            });
        }
        match response::parse_results(text.as_deref(), targets.len()) {
            Ok(results) => Ok(CompressOutcome::Results(results)),
            Err(failure) => self.failed(failure),
        }
    }

    fn request_headers(&self) -> Result<HeaderMap, Failure> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut api_key =
            HeaderValue::from_str(self.api_key.expose_secret()).map_err(|source| Failure {
                message: REQUEST_FAILED,
                detail: source.to_string(),
            })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-api-key"), api_key);
        Ok(headers)
    }

    fn failed(&self, failure: Failure) -> Result<CompressOutcome, CompresrTransportError> {
        if self.settings.fallback == UnreachableFallback::FailOpen {
            tracing::warn!(
                detail = %failure.detail,
                "Compresr: {}; fail_open configured, forwarding request uncompressed",
                failure.message
            );
            return Ok(CompressOutcome::Uncompressed);
        }
        tracing::error!(detail = %failure.detail, "Compresr: {}", failure.message);
        Err(CompresrTransportError::CompressionFailed {
            message: failure.message,
        })
    }
}

// LiteLLM strips these at init, so a passthrough key can never replace the content it would compress
fn without_reserved_params(
    params: BTreeMap<String, GuardrailValue>,
) -> BTreeMap<String, GuardrailValue> {
    let reserved: Vec<&str> = params
        .keys()
        .filter(|key| RESERVED_COMPRESSION_PARAM_KEYS.contains(&key.as_str()))
        .map(String::as_str)
        .collect();
    if reserved.is_empty() {
        return params;
    }
    tracing::warn!(
        keys = ?reserved,
        "Compresr: ignoring reserved compression_params keys"
    );
    params
        .into_iter()
        .filter(|(key, _)| !RESERVED_COMPRESSION_PARAM_KEYS.contains(&key.as_str()))
        .collect()
}

impl CompressionResult {
    pub(super) fn from_fields(fields: &BTreeMap<String, GuardrailValue>) -> Self {
        Self {
            compressed_context: match fields.get("compressed_context") {
                Some(GuardrailValue::Text(text)) => Some(text.clone()),
                _ => None,
            },
            original_tokens: response::safe_int(fields.get("original_tokens")),
            compressed_tokens: response::safe_int(fields.get("compressed_tokens")),
        }
    }
}
