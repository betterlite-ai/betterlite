use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_API_BASE: &str = "https://server.lasso.security/gateway/v3";
const CACHE_TTL_SECONDS: u64 = 3_600;

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
        response: Value,
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

impl std::fmt::Debug for LassoGuardrail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LassoGuardrail")
            .field("api_base", &self.api_base)
            .field("user_id", &self.user_id)
            .field("conversation_id", &self.conversation_id)
            .field("mask", &self.mask)
            .finish_non_exhaustive()
    }
}

impl LassoGuardrail {
    pub fn new(
        api_key: Option<String>,
        api_base: Option<String>,
        user_id: Option<String>,
        conversation_id: Option<String>,
        mask: bool,
    ) -> Result<Self, LassoError> {
        let api_key = api_key.ok_or(LassoError::MissingSecrets)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|source| LassoError::Request { source })?;
        Ok(Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            user_id,
            conversation_id,
            mask,
            client,
        })
    }

    pub fn conversation_id<C: ConversationCache>(
        &self,
        litellm_call_id: Option<&str>,
        cache: &C,
    ) -> String {
        if let Some(conversation_id) = self.conversation_id.as_deref() {
            return conversation_id.to_owned();
        }
        let Some(call_id) = litellm_call_id.filter(|value| !value.is_empty()) else {
            return Self::generate_conversation_id();
        };
        let key = format!("lasso_conversation_id:{call_id}");
        if let Ok(Some(value)) = cache.get_cache(&key) {
            return value;
        }
        let generated = Self::generate_conversation_id();
        let _ = cache.set_cache(&key, &generated, CACHE_TTL_SECONDS);
        generated
    }

    pub fn prepare_headers<C: ConversationCache>(
        &self,
        litellm_call_id: Option<&str>,
        cache: &C,
    ) -> Result<HeaderMap, LassoError> {
        let conversation_id = self.conversation_id(litellm_call_id, cache);
        let mut headers = HeaderMap::new();
        let api_key =
            HeaderValue::from_str(&self.api_key).map_err(|_| LassoError::MissingSecrets)?;
        let conversation =
            HeaderValue::from_str(&conversation_id).map_err(|_| LassoError::Cache {
                message: "invalid conversation ID".to_owned(),
            })?;
        headers.insert(HeaderName::from_static("lasso-api-key"), api_key);
        headers.insert(
            HeaderName::from_static("lasso-conversation-id"),
            conversation,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(user_id) = self.user_id.as_deref() {
            let value = HeaderValue::from_str(user_id).map_err(|_| LassoError::Cache {
                message: "invalid user ID".to_owned(),
            })?;
            headers.insert(HeaderName::from_static("lasso-user-id"), value);
        }
        Ok(headers)
    }

    pub fn prepare_payload<C: ConversationCache>(
        &self,
        messages: Vec<Value>,
        data: &Value,
        message_type: &str,
        cache: &C,
    ) -> Value {
        let call_id = data.get("litellm_call_id").and_then(Value::as_str);
        let session_id = self.conversation_id(call_id, cache);
        let mut payload = Map::new();
        payload.insert("messages".to_owned(), Value::Array(messages));
        payload.insert(
            "messageType".to_owned(),
            Value::String(message_type.to_owned()),
        );
        payload.insert("sessionId".to_owned(), Value::String(session_id));
        payload.insert("source".to_owned(), serde_json::json!({"type": "litellm"}));
        if let Some(user_id) = self.user_id.as_deref() {
            payload.insert("userId".to_owned(), Value::String(user_id.to_owned()));
        }
        let tools = flatten_tools(data.get("tools"));
        if !tools.is_empty() {
            payload.insert("tools".to_owned(), Value::Array(tools));
        }
        Value::Object(payload)
    }

    pub async fn call_api(
        &self,
        headers: HeaderMap,
        payload: &Value,
        api_url: Option<&str>,
    ) -> Result<Value, LassoError> {
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

    pub fn process_response(&self, response: Value) -> Result<(), LassoError> {
        if response.get("violations_detected") != Some(&Value::Bool(true)) {
            return Ok(());
        }
        let deputies = blocking_deputies(&response);
        if deputies.is_empty() {
            return Ok(());
        }
        Err(LassoError::Blocked { deputies, response })
    }

    fn generate_conversation_id() -> String {
        Uuid::new_v4().to_string()
    }
}

fn flatten_tools(value: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(tools)) = value else {
        return Vec::new();
    };
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?.as_object()?;
            let name = function.get("name")?.clone();
            let mut definition = Map::new();
            definition.insert("name".to_owned(), name);
            for field in ["description", "parameters"] {
                if let Some(value) = function.get(field) {
                    definition.insert(field.to_owned(), value.clone());
                }
            }
            Some(Value::Object(definition))
        })
        .collect()
}

pub fn extract_tool_arguments(tool_call: &Value) -> Option<Value> {
    let arguments = tool_call.get("function")?.get("arguments")?;
    let raw = arguments.as_str()?;
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(object)) => Some(Value::Object(object)),
        _ => Some(serde_json::json!({"arguments": raw})),
    }
}

fn blocking_deputies(response: &Value) -> Vec<String> {
    let Some(findings) = response.get("findings").and_then(Value::as_object) else {
        return Vec::new();
    };
    findings
        .iter()
        .filter_map(|(deputy, entries)| {
            let blocked = entries
                .as_array()?
                .iter()
                .any(|entry| entry.get("action").and_then(Value::as_str) == Some("BLOCK"));
            blocked.then(|| deputy.clone())
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCacheEntry {
    pub value: String,
    pub ttl_seconds: u64,
}

impl ConversationCache for std::cell::RefCell<BTreeMap<String, MemoryCacheEntry>> {
    fn get_cache(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.borrow().get(key).map(|entry| entry.value.clone()))
    }

    fn set_cache(&self, key: &str, value: &str, ttl_seconds: u64) -> Result<(), String> {
        self.borrow_mut().insert(
            key.to_owned(),
            MemoryCacheEntry {
                value: value.to_owned(),
                ttl_seconds,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
