pub mod error;
pub mod init;
pub mod wire;

mod tests;

use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use self::error::PromptGuardError;
use self::init::PromptGuardArgs;
use self::wire::{GuardRequest, GuardResponse};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;

pub const DEFAULT_API_BASE: &str = "https://api.promptguard.co";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const SUPPORTED_EVENT_HOOKS: [&str; 2] = ["pre_call", "post_call"];
pub const API_KEY_VARIABLE: &str = "PROMPTGUARD_API_KEY";
pub const API_BASE_VARIABLE: &str = "PROMPTGUARD_API_BASE";
pub const BLOCK_ON_ERROR_VARIABLE: &str = "PROMPTGUARD_BLOCK_ON_ERROR";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct PromptGuardGuardrail {
    pub name: String,
    pub api_base: String,
    pub api_key: SecretString,
    pub block_on_error: bool,
    pub timeout: Duration,
    pub client: reqwest::Client,
}

impl PromptGuardGuardrail {
    pub fn new<L>(
        args: PromptGuardArgs,
        env: &L,
        client: reqwest::Client,
    ) -> Result<Self, PromptGuardError>
    where
        L: EnvironmentLookup,
    {
        let api_key = args
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| env_api_key(env))
            .ok_or(PromptGuardError::MissingCredentials)?;
        let api_base = args
            .api_base
            .filter(|base| !base.is_empty())
            .or_else(|| {
                env.lookup(API_BASE_VARIABLE)
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let block_on_error = args
            .block_on_error
            .map_or_else(|| env_block_on_error(env), Ok)?;
        let timeout = args.timeout.map_or(Ok(DEFAULT_TIMEOUT), |seconds| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|source| PromptGuardError::UnrepresentableTimeout { source })
        })?;
        Ok(Self {
            name: args.name,
            api_base,
            api_key,
            block_on_error,
            timeout,
            client,
        })
    }

    pub fn request_headers(&self) -> Result<HeaderMap, PromptGuardError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let mut api_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|source| PromptGuardError::InvalidHeader { source })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-api-key"), api_key);
        Ok(headers)
    }

    pub async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, PromptGuardError> {
        let Some(request) = wire::build_request(&inputs, input_type) else {
            return Ok(inputs);
        };
        match self.post(&request).await {
            Ok(response) => wire::apply_decision(inputs, response),
            Err(error) => {
                tracing::error!(error = %error, "PromptGuard API error");
                if self.block_on_error {
                    Err(error)
                } else {
                    Ok(inputs)
                }
            }
        }
    }

    async fn post(&self, request: &GuardRequest) -> Result<GuardResponse, PromptGuardError> {
        let endpoint = format!("{}{}", self.api_base, wire::GUARD_ENDPOINT);
        let response = self
            .client
            .post(endpoint)
            .headers(self.request_headers()?)
            .json(request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(PromptGuardError::HttpStatus { status });
        }
        let body = read_body(response).await?;
        serde_json::from_slice(&body).map_err(|source| PromptGuardError::ResponseJson { source })
    }
}

fn env_api_key<L>(env: &L) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    env.lookup(API_KEY_VARIABLE)
        .filter(|key| !key.is_empty())
        .map(SecretString::from)
}

fn env_block_on_error<L>(env: &L) -> Result<bool, PromptGuardError>
where
    L: EnvironmentLookup,
{
    let Some(text) = env.lookup(BLOCK_ON_ERROR_VARIABLE) else {
        return Ok(true);
    };
    betterlite_env::flag(BLOCK_ON_ERROR_VARIABLE, &text)
        .map(|flag| flag.unwrap_or(true))
        .map_err(|source| PromptGuardError::Environment { source })
}

fn request_error(source: reqwest::Error) -> PromptGuardError {
    PromptGuardError::Request {
        source: source.without_url(),
    }
}

async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>, PromptGuardError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(PromptGuardError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
