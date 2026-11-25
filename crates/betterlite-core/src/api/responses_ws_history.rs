pub mod call_kwargs;
pub mod error;
pub mod history;
pub mod messages;
pub mod provider;
pub mod serialization;
pub mod warmup;

mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::error::{FrameSinkError, ResponsesWsHistoryError};
use self::provider::ProviderResolution;
use self::serialization::{ChunkWriter, INVALID_JSON_MESSAGE, INVALID_REQUEST_ERROR, send_error};
use crate::api::response_synthetic_events::chunks::payload_bytes;
use crate::api::response_synthetic_events::python_value::has_content;
use crate::api::responses_stream_events::ProjectQuotaCallback;

pub const MANAGED_WS_SKIP_KWARGS: [&str; 5] = [
    "litellm_logging_obj",
    "litellm_call_id",
    "aresponses",
    "_aresponses_websocket",
    "user_api_key_dict",
];

pub const SESSION_HISTORY_BUDGET_BYTES: usize = 16 * 1024 * 1024;

#[async_trait]
pub trait FrameSink: Send + Sync {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError>;
}

pub struct Connection<Sink: FrameSink> {
    pub websocket: Sink,
    pub model: String,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub user_api_key_dict: Option<UserAPIKeyAuth>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<String>,
    pub first_message: Option<String>,
    pub quota_callbacks: Option<Vec<Arc<dyn ProjectQuotaCallback>>>,
    pub kwargs: IndexMap<String, Json>,
}

pub struct WsDeps {
    pub resolver: Arc<dyn ProviderResolution>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub ids: Arc<dyn Fn() -> String + Send + Sync>,
    pub serializer: Arc<ChunkWriter>,
}

pub struct ManagedResponsesWebSocket<Sink: FrameSink> {
    pub websocket: Sink,
    pub model: String,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub user_api_key_dict: Option<UserAPIKeyAuth>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub model_group: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<String>,
    pub connection_provider: Option<String>,
    pub first_message: Option<String>,
    pub quota_callbacks: Vec<Arc<dyn ProjectQuotaCallback>>,
    pub extra_kwargs: IndexMap<String, Json>,
    pub session_history: IndexMap<String, Vec<IndexMap<String, Json>>>,
    // the serialized JSON length of every stored list, lowered as entries leave, so a store never re-sums the map
    pub session_history_bytes: usize,
    pub session_history_budget_bytes: usize,
    pub resolver: Arc<dyn ProviderResolution>,
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub ids: Arc<dyn Fn() -> String + Send + Sync>,
    pub serializer: Arc<ChunkWriter>,
}

impl<Sink: FrameSink> ManagedResponsesWebSocket<Sink> {
    #[must_use]
    pub fn new(connection: Connection<Sink>, dependencies: WsDeps) -> Self {
        let litellm_metadata = connection.litellm_metadata.unwrap_or_default();
        let model_group = model_group_of(&litellm_metadata);
        let connection_provider = resolve_connection_provider(
            dependencies.resolver.as_ref(),
            &connection.model,
            connection.custom_llm_provider.as_deref(),
        );
        Self {
            websocket: connection.websocket,
            model: connection.model,
            logging_obj: connection.logging_obj,
            user_api_key_dict: connection.user_api_key_dict,
            litellm_metadata,
            model_group,
            api_key: connection.api_key,
            api_base: connection.api_base,
            timeout: connection.timeout,
            custom_llm_provider: connection.custom_llm_provider,
            connection_provider,
            first_message: connection.first_message,
            quota_callbacks: connection.quota_callbacks.unwrap_or_default(),
            extra_kwargs: filtered_kwargs(connection.kwargs),
            session_history: IndexMap::new(),
            session_history_bytes: 0,
            session_history_budget_bytes: SESSION_HISTORY_BUDGET_BYTES,
            resolver: dependencies.resolver,
            clock: dependencies.clock,
            ids: dependencies.ids,
            serializer: dependencies.serializer,
        }
    }

    // LiteLLM's _store_history keeps every turn; one connection's history stays inside its byte budget
    pub fn store_history(&mut self, response_id: String, messages: Vec<IndexMap<String, Json>>) {
        let bytes = payload_bytes(&messages);
        if bytes > self.session_history_budget_bytes {
            tracing::warn!(
                "ManagedResponsesWS: not storing {bytes} bytes of history for response_id={response_id}; the budget is {} bytes",
                self.session_history_budget_bytes
            );
            return;
        }
        if let Some(replaced) = self.session_history.shift_remove(&response_id) {
            self.session_history_bytes = self
                .session_history_bytes
                .saturating_sub(payload_bytes(&replaced));
        }
        while self.session_history_bytes.saturating_add(bytes) > self.session_history_budget_bytes {
            let Some((_stale_id, stale)) = self.session_history.shift_remove_index(0) else {
                break;
            };
            self.session_history_bytes = self
                .session_history_bytes
                .saturating_sub(payload_bytes(&stale));
        }
        let stored = messages.len();
        self.session_history.insert(response_id.clone(), messages);
        self.session_history_bytes = self.session_history_bytes.saturating_add(bytes);
        tracing::debug!(
            "ManagedResponsesWS: stored {stored} messages for response_id={response_id}"
        );
    }

    pub async fn parse_message(
        &self,
        raw_message: &str,
    ) -> Result<Option<IndexMap<String, Json>>, ResponsesWsHistoryError> {
        let Ok(message) = serde_json::from_str::<Json>(raw_message) else {
            send_error(self, INVALID_JSON_MESSAGE, Some(INVALID_REQUEST_ERROR)).await;
            return Ok(None);
        };
        let Json::Object(fields) = message else {
            return Err(ResponsesWsHistoryError::Shape {
                field: "message",
                expected: "a JSON object",
            });
        };
        match fields.get("type") {
            Some(Json::Text(message_type)) if message_type == "response.create" => Ok(Some(fields)),
            _ => Ok(None),
        }
    }
}

fn model_group_of(litellm_metadata: &IndexMap<String, Json>) -> Option<String> {
    let raw = litellm_metadata
        .get("model_group")
        .filter(|value| has_content(value))
        .or_else(|| litellm_metadata.get("deployment_model_name"))?;
    match raw {
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn resolve_connection_provider(
    resolver: &dyn ProviderResolution,
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Option<String> {
    if model.is_empty() {
        return custom_llm_provider.map(str::to_owned);
    }
    resolver
        .resolve(model)
        .ok()
        .filter(|provider| !provider.is_empty())
        .or_else(|| custom_llm_provider.map(str::to_owned))
}

fn filtered_kwargs(mut kwargs: IndexMap<String, Json>) -> IndexMap<String, Json> {
    kwargs.retain(|key, _value| !MANAGED_WS_SKIP_KWARGS.contains(&key.as_str()));
    kwargs
}
