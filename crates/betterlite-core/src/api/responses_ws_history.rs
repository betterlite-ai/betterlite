pub mod call_kwargs;
pub mod error;
pub mod history;
pub mod messages;
pub mod provider;
pub mod serialization;
pub mod warmup;

mod values;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::error::{FrameSinkError, ResponsesWsHistoryError};
use self::provider::ProviderResolution;
use self::serialization::{ChunkWriter, INVALID_JSON_MESSAGE, INVALID_REQUEST_ERROR};
use self::values::metadata_model_group;
use crate::api::responses_stream_events::ProjectQuotaCallback;

pub const MANAGED_WS_SKIP_KWARGS: [&str; 5] = [
    "litellm_logging_obj",
    "litellm_call_id",
    "aresponses",
    "_aresponses_websocket",
    "user_api_key_dict",
];

// ResponsesClientWebSocket: the client-facing connection this handler writes text frames to
#[async_trait]
pub trait FrameSink: Send + Sync {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError>;
}

// The arguments LiteLLM's constructor receives, with the kwargs map kept apart from the typed ones
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

// The seams the handler does not own: get_llm_provider, the warmup clock and uuid, and chunk serialization
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
    session_history: HashMap<String, Vec<IndexMap<String, Json>>>,
    resolver: Arc<dyn ProviderResolution>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ids: Arc<dyn Fn() -> String + Send + Sync>,
    serializer: Arc<ChunkWriter>,
}

impl<Sink: FrameSink> ManagedResponsesWebSocket<Sink> {
    // __init__: store the connection arguments and pick the provider once for the whole connection
    #[must_use]
    pub fn new(connection: Connection<Sink>, dependencies: WsDeps) -> Self {
        let litellm_metadata = connection.litellm_metadata.unwrap_or_else(IndexMap::new);
        let model_group = metadata_model_group(&litellm_metadata);
        let resolved = if connection.model.is_empty() {
            None
        } else {
            dependencies
                .resolver
                .resolve(&connection.model)
                .ok()
                .filter(|provider| !provider.is_empty())
        };
        let connection_provider = resolved.or_else(|| connection.custom_llm_provider.clone());
        let extra_kwargs = connection
            .kwargs
            .into_iter()
            .filter(|(key, _value)| !MANAGED_WS_SKIP_KWARGS.contains(&key.as_str()))
            .collect();
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
            quota_callbacks: connection.quota_callbacks.unwrap_or_else(Vec::new),
            extra_kwargs,
            session_history: HashMap::new(),
            resolver: dependencies.resolver,
            clock: dependencies.clock,
            ids: dependencies.ids,
            serializer: dependencies.serializer,
        }
    }

    // _parse_message: bad JSON becomes one error frame, other valid JSON is not this handler's message
    pub async fn parse_message(
        &self,
        raw_message: &str,
    ) -> Result<Option<IndexMap<String, Json>>, ResponsesWsHistoryError> {
        let Ok(message) = serde_json::from_str::<Json>(raw_message) else {
            self.send_error(INVALID_JSON_MESSAGE, Some(INVALID_REQUEST_ERROR))
                .await;
            return Ok(None);
        };
        let Json::Object(fields) = message else {
            return Err(ResponsesWsHistoryError::Shape {
                field: "message",
                expected: "a JSON object",
            });
        };
        let is_response_create = matches!(
            fields.get("type"),
            Some(Json::Text(message_type)) if message_type == "response.create"
        );
        if !is_response_create {
            return Ok(None);
        }
        Ok(Some(fields))
    }
}
