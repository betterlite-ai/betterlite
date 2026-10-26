use std::sync::Arc;
use std::time::Duration;

use betterlite_config::types::ModelDeployment;
use betterlite_http::error::HttpError;
use betterlite_http::outbound::OutboundClient;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

pub type Object = Map<String, Value>;

pub trait RandomSource: Send + Sync {
    fn next(&self, upper_bound: usize) -> usize;
}

#[derive(Debug, Default)]
pub struct SystemRandom;

impl RandomSource for SystemRandom {
    fn next(&self, upper_bound: usize) -> usize {
        let sample = Uuid::new_v4().as_u128();
        #[expect(
            clippy::arithmetic_side_effects,
            clippy::integer_division_remainder_used,
            reason = "deployment selection needs a uniform bounded sample"
        )]
        usize::try_from(sample % u128::try_from(upper_bound).unwrap_or(1)).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("the requested model is not configured")]
    UnknownModel,
    #[error("the configured model is not an OpenAI model")]
    UnsupportedProvider,
    #[error("the upstream URL is invalid: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the upstream request timed out")]
    Timeout,
    #[error("the upstream returned status {status}")]
    Provider { status: u16, body: String },
    #[error("the upstream response is invalid: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the HTTP client failed: {source}")]
    Http {
        #[source]
        source: HttpError,
    },
}

pub struct ChatExecutor {
    client: OutboundClient,
    environment_key: Option<SecretString>,
    catalog: CatalogCapabilities,
    random: Arc<dyn RandomSource>,
}

impl ChatExecutor {
    #[must_use]
    pub fn new(
        client: OutboundClient,
        environment_key: Option<SecretString>,
        catalog: CatalogCapabilities,
    ) -> Self {
        Self {
            client,
            environment_key,
            catalog,
            random: Arc::new(SystemRandom),
        }
    }

    #[must_use]
    pub fn with_random(mut self, random: Arc<dyn RandomSource>) -> Self {
        self.random = random;
        self
    }

    pub async fn complete(
        &self,
        request: &ChatRequest,
        deployments: &[ModelDeployment],
        request_timeout: Option<f64>,
    ) -> Result<ChatResponse, ChatError> {
        if request.stream {
            return Err(ChatError::Provider {
                status: 400,
                body: "streaming is not supported".to_owned(),
            });
        }
        let candidates: Vec<&ModelDeployment> = deployments
            .iter()
            .filter(|item| item.model_name == request.model)
            .collect();
        let deployment = candidates
            .get(self.random.next(candidates.len()))
            .copied()
            .ok_or(ChatError::UnknownModel)?;
        let params = &deployment.litellm_params;
        let provider_model = params
            .model
            .as_deref()
            .ok_or(ChatError::UnsupportedProvider)?;
        let model = provider_model
            .strip_prefix("openai/")
            .ok_or(ChatError::UnsupportedProvider)?;
        let api_key = params.api_key.as_ref().or(self.environment_key.as_ref());
        let base = params
            .api_base
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");
        let url = endpoint(base)?;
        let timeout = duration(params.timeout.or(request_timeout).unwrap_or(6_000.0_f64));
        let body = self.body(request, model);
        self.send(url, api_key, timeout, body).await
    }

    async fn send(
        &self,
        url: Url,
        api_key: Option<&SecretString>,
        timeout: Duration,
        body: Object,
    ) -> Result<ChatResponse, ChatError> {
        let response = self
            .client
            .send(self.client.post_json(url, api_key, timeout, &body))
            .await
            .map_err(|source| {
                if is_timeout(&source) {
                    ChatError::Timeout
                } else {
                    ChatError::Http { source }
                }
            })?;
        let status = response.status().as_u16();
        let bytes = response.bytes().await.map_err(|source| ChatError::Http {
            source: HttpError::Request(source),
        })?;
        if status >= 400 {
            let provider_body = String::from_utf8(bytes.to_vec())
                .unwrap_or_else(|_| "upstream returned a non-UTF-8 error".to_owned());
            return Err(ChatError::Provider {
                status,
                body: provider_body,
            });
        }
        serde_json::from_slice(&bytes).map_err(|source| ChatError::Decode { source })
    }

    fn body(&self, request: &ChatRequest, model: &str) -> Object {
        let mut requested =
            serde_json::to_value(request).unwrap_or_else(|_| Value::Object(Object::new()));
        if let Value::Object(ref mut object) = requested {
            object.insert("model".to_owned(), Value::String(model.to_owned()));
            object.remove("extra");
        }
        let mut body = Object::new();
        let config = OpenAiConfig {
            family: ConfigFamily::OpenAi,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        };
        config.map_params(
            model,
            &requested.as_object().cloned().unwrap_or_default(),
            &mut body,
            &self.catalog,
        );
        body.insert("model".to_owned(), Value::String(model.to_owned()));
        body.insert(
            "messages".to_owned(),
            serde_json::to_value(&request.messages).unwrap_or(Value::Array(Vec::new())),
        );
        body.insert("stream".to_owned(), Value::Bool(false));
        body
    }
}

fn endpoint(base: &str) -> Result<Url, ChatError> {
    let mut url = Url::parse(base).map_err(|source| ChatError::Url { source })?;
    let path = format!("{}/chat/completions", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn duration(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(seconds.max(0.001)).unwrap_or(Duration::from_mins(100))
}

fn is_timeout(error: &HttpError) -> bool {
    matches!(error, HttpError::Request(source) if source.is_timeout())
}

#[cfg(test)]
mod tests;
