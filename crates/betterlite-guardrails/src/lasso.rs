use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::value::RawValue;
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_API_BASE: &str = "https://server.lasso.security/gateway/v3";
const CACHE_TTL_SECONDS: u64 = 3_600;

pub mod wire;
use wire::{
    LassoPayload, LassoRequestData, LassoResponse, MessageType, OpenAiTool, Source, ToolArguments,
    ToolDefinition,
};

pub trait ConversationCache {
    fn get_cache(&self, key: &str) -> Result<Option<String>, String>;
    fn set_cache(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<(), String>;
}

#[derive(Debug, Error)]
pub enum LassoError {
    #[error("Lasso API key is required")]
    MissingSecrets,
    #[error("Lasso cache error: {message}")]
    Cache { message: String },
    #[error("Lasso request failed: {source}")]
    Request { source: reqwest::Error },
    #[error("Lasso returned HTTP status {status}")]
    HttpStatus {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("Lasso response JSON is invalid: {source}")]
    ResponseJson { source: serde_json::Error },
    #[error("Lasso blocked the request: {deputies:?}")]
    Blocked {
        deputies: Vec<String>,
        response: LassoResponse,
    },
}

#[derive(Clone)]
pub struct LassoGuardrail {
    api_key: String,
    api_base: String,
    user_id: Option<String>,
    conversation_id: Option<String>,
    mask: bool,
    client: reqwest::Client,
}

impl Debug for LassoGuardrail {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LassoGuardrail")
            .field("api_base", &self.api_base)
            .field("user_id", &self.user_id)
            .field("conversation_id", &self.conversation_id)
            .field("mask", &self.mask)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Default)]
pub struct LassoSettings {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mask: bool,
}

impl LassoGuardrail {
    pub fn new(settings: LassoSettings, client: reqwest::Client) -> Result<Self, LassoError> {
        let resolved_api_key = settings.api_key.ok_or(LassoError::MissingSecrets)?;
        Ok(Self {
            api_key: resolved_api_key,
            api_base: settings
                .api_base
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            user_id: settings.user_id,
            conversation_id: settings.conversation_id,
            mask: settings.mask,
            client,
        })
    }

    pub fn conversation_id<C>(&self, litellm_call_id: Option<&str>, cache: &C) -> String
    where
        C: ConversationCache,
    {
        if let Some(conversation_id) = self.conversation_id.as_deref() {
            return conversation_id.to_owned();
        }
        let Some(call_id) = litellm_call_id.filter(|value| !value.is_empty()) else {
            return Uuid::new_v4().to_string();
        };
        let key = format!("lasso_conversation_id:{call_id}");
        if let Ok(Some(value)) = cache.get_cache(&key) {
            return value;
        }
        let generated = Uuid::new_v4().to_string();
        drop(cache.set_cache(&key, &generated, CACHE_TTL_SECONDS));
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
        let conversation_id = self.conversation_id(litellm_call_id, cache);
        let api_key =
            HeaderValue::from_str(&self.api_key).map_err(|invalid| LassoError::Cache {
                message: format!("invalid API key header: {invalid}"),
            })?;
        let conversation =
            HeaderValue::from_str(&conversation_id).map_err(|invalid| LassoError::Cache {
                message: format!("invalid conversation ID header: {invalid}"),
            })?;
        let mut headers = HeaderMap::new();
        headers.insert(HeaderName::from_static("lasso-api-key"), api_key);
        headers.insert(
            HeaderName::from_static("lasso-conversation-id"),
            conversation,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(user_id) = self.user_id.as_deref() {
            let value = HeaderValue::from_str(user_id).map_err(|invalid| LassoError::Cache {
                message: format!("invalid user ID header: {invalid}"),
            })?;
            headers.insert(HeaderName::from_static("lasso-user-id"), value);
        }
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
        let session_id = self.conversation_id(data.litellm_call_id.as_deref(), cache);
        LassoPayload {
            messages,
            message_type,
            session_id,
            user_id: self.user_id.clone(),
            tools: data
                .tools
                .iter()
                .filter_map(|tool| {
                    let function = tool.function.as_ref()?;
                    let name = function.name.clone()?;
                    Some(ToolDefinition {
                        name,
                        description: function.description.clone(),
                        parameters: function.parameters.clone(),
                    })
                })
                .collect(),
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
        let endpoint = api_url.map_or_else(
            || {
                format!(
                    "{}/{}",
                    self.api_base.trim_end_matches('/'),
                    if self.mask { "classifix" } else { "classify" }
                )
            },
            ToOwned::to_owned,
        );
        let response = self
            .client
            .post(endpoint)
            .headers(headers)
            .json(payload)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|source| LassoError::Request { source })?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|source| LassoError::Request { source })?;
        if !status.is_success() {
            return Err(LassoError::HttpStatus { status, body });
        }
        serde_json::from_str(&body).map_err(|source| LassoError::ResponseJson { source })
    }

    pub fn process_response(&self, response: LassoResponse) -> Result<(), LassoError> {
        if !response.violations_detected {
            return Ok(());
        }
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
            return Ok(());
        }
        Err(LassoError::Blocked { deputies, response })
    }
}

#[must_use]
pub fn extract_tool_arguments(tool_call: &OpenAiTool) -> Option<ToolArguments> {
    let raw = tool_call.function.as_ref()?.arguments.as_deref()?;
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(raw).map_or_else(
        |_| {
            Some(ToolArguments::Malformed {
                arguments: raw.to_owned(),
            })
        },
        |parsed| Some(ToolArguments::Parsed(parsed)),
    )
}

#[path = "lasso/tests.rs"]
mod tests;
