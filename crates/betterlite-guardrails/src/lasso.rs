use std::error::Error as StdError;
use std::time::Duration;

use betterlite_config::sections::lasso_classification::Configured;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use ulid::Ulid;

pub mod error;
pub mod wire;
use error::LassoError;
use wire::{
    LassoPayload, LassoRequestData, LassoResponse, MessageType, Source, ToolArguments, ToolCall,
    ToolCallFields, ToolDefinition,
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const CONVERSATION_TTL: Duration = Duration::from_secs(3_600);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub trait ConversationCache {
    type Error: StdError;

    fn get_cache(&self, key: &str) -> Result<Option<String>, Self::Error>;
    fn set_cache(&self, key: &str, value: &str, ttl: Duration) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone)]
pub struct LassoGuardrail {
    api_key: SecretString,
    api_base: String,
    user_id: Option<String>,
    conversation_id: Option<String>,
    client: reqwest::Client,
    timeout: Duration,
    new_id: fn() -> Ulid,
}

impl LassoGuardrail {
    #[must_use]
    pub fn new(
        settings: &Configured,
        client: reqwest::Client,
        timeout: Duration,
        new_id: fn() -> Ulid,
    ) -> Self {
        Self {
            api_key: settings.api_key.clone(),
            api_base: settings.api_base.clone(),
            user_id: settings.user_id.clone(),
            conversation_id: settings.conversation_id.clone(),
            client,
            timeout,
            new_id,
        }
    }

    pub fn conversation_id<C>(&self, litellm_call_id: Option<&str>, cache: &C) -> String
    where
        C: ConversationCache,
    {
        if let Some(configured) = self.conversation_id.as_deref() {
            return configured.to_owned();
        }
        let Some(call_id) = litellm_call_id.filter(|id| !id.is_empty()) else {
            return (self.new_id)().to_string();
        };
        let key = format!("lasso_conversation_id:{call_id}");
        match cache.get_cache(&key) {
            Ok(Some(cached)) if !cached.is_empty() => return cached,
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "Lasso conversation ID cache read failed"),
        }
        let generated = (self.new_id)().to_string();
        if let Err(error) = cache.set_cache(&key, &generated, CONVERSATION_TTL) {
            tracing::warn!(%error, "Lasso conversation ID cache write failed");
        }
        generated
    }

    pub fn prepare_headers<C>(
        &self,
        litellm_call_id: Option<&str>,
        cache: &C,
    ) -> Result<HeaderMap, LassoError>
    where
        C: ConversationCache,
    {
        let mut headers = HeaderMap::new();
        let mut api_key =
            HeaderValue::from_str(self.api_key.expose_secret()).map_err(|source| {
                LassoError::InvalidHeader {
                    name: "lasso-api-key",
                    source,
                }
            })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("lasso-api-key"), api_key);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(user_id) = self.user_id.as_deref() {
            let value =
                HeaderValue::from_str(user_id).map_err(|source| LassoError::InvalidHeader {
                    name: "lasso-user-id",
                    source,
                })?;
            headers.insert(HeaderName::from_static("lasso-user-id"), value);
        }
        let conversation_id = self.conversation_id(litellm_call_id, cache);
        let conversation = HeaderValue::from_str(&conversation_id).map_err(|source| {
            LassoError::InvalidHeader {
                name: "lasso-conversation-id",
                source,
            }
        })?;
        headers.insert(
            HeaderName::from_static("lasso-conversation-id"),
            conversation,
        );
        Ok(headers)
    }

    pub fn prepare_payload<C>(
        &self,
        messages: Vec<Box<RawValue>>,
        data: &LassoRequestData,
        message_type: MessageType,
        cache: &C,
    ) -> LassoPayload
    where
        C: ConversationCache,
    {
        let tools = data
            .tools
            .iter()
            .filter_map(|tool| {
                let function = tool.function.as_ref()?;
                Some(ToolDefinition {
                    name: function.name.clone().filter(|name| !name.is_empty())?,
                    description: function.description.clone().filter(|text| !text.is_empty()),
                    parameters: function
                        .parameters
                        .clone()
                        .filter(|schema| !schema.is_empty()),
                })
            })
            .collect();
        LassoPayload {
            messages,
            message_type,
            session_id: self.conversation_id(data.litellm_call_id.as_deref(), cache),
            user_id: self.user_id.clone(),
            tools,
            source: Source {
                source_type: "litellm".to_owned(),
            },
        }
    }

    pub async fn call_api(
        &self,
        headers: HeaderMap,
        payload: &LassoPayload,
        api_url: Option<&str>,
    ) -> Result<LassoResponse, LassoError> {
        let endpoint =
            api_url.map_or_else(|| format!("{}/classify", self.api_base), ToOwned::to_owned);
        let mut response = self
            .client
            .post(endpoint)
            .headers(headers)
            .json(payload)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(LassoError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|source| LassoError::ResponseJson { source })
    }

    pub fn process_response(&self, response: LassoResponse) -> Result<(), LassoError> {
        if !response.violations_detected {
            return Ok(());
        }
        let violated: Vec<&str> = response
            .deputies
            .iter()
            .filter(|(_, violated)| **violated)
            .map(|(deputy, _)| deputy.as_str())
            .collect();
        tracing::warn!(?violated, "Lasso guardrail detected violations");
        let deputies: Vec<String> = response
            .findings
            .iter()
            .filter(|(_, findings)| {
                findings
                    .iter()
                    .any(|finding| finding.action.as_deref() == Some("BLOCK"))
            })
            .map(|(deputy, _)| deputy.clone())
            .collect();
        if deputies.is_empty() {
            tracing::info!(?violated, "Lasso violations do not block, continuing");
            return Ok(());
        }
        Err(LassoError::Blocked { deputies, response })
    }
}

fn request_error(source: reqwest::Error) -> LassoError {
    if source.is_timeout() {
        LassoError::Timeout { source }
    } else {
        LassoError::Request { source }
    }
}

fn status_error(status: StatusCode) -> LassoError {
    match status {
        StatusCode::UNAUTHORIZED => LassoError::InvalidApiKey,
        StatusCode::TOO_MANY_REQUESTS => LassoError::RateLimited,
        _ => LassoError::HttpStatus { status },
    }
}

#[must_use]
pub fn extract_tool_call_fields(call: &ToolCall) -> ToolCallFields {
    let Some(function) = call.function.as_ref() else {
        return ToolCallFields {
            id: call.id.clone(),
            name: None,
            input: None,
        };
    };
    // a call that smuggles content as malformed JSON still reaches Lasso as the raw string
    let input = function
        .arguments
        .as_deref()
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            serde_json::from_str(raw).map_or_else(
                |_| ToolArguments::Malformed {
                    arguments: raw.to_owned(),
                },
                ToolArguments::Parsed,
            )
        });
    ToolCallFields {
        id: call.id.clone(),
        name: function.name.clone(),
        input,
    }
}

#[path = "lasso/tests.rs"]
mod tests;
