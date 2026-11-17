pub mod collect;
pub mod error;
pub mod logging;
pub mod records;
pub mod store;
mod tests;
pub mod usage;

use std::fmt;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::realtime_contracts::events::{AllDeltaType, AudioTranscriptionUsage};
use super::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeEventNormalizer,
};
use logging::{RealtimeLogging, RealtimeSuccessQueue};

pub const DEFAULT_LOGGED_EVENT_TYPES: [&str; 5] = [
    "session.created",
    "response.create",
    "response.done",
    "conversation.item.added",
    "conversation.item.done",
];
pub const TRANSCRIPTION_COMPLETED: &str = "conversation.item.input_audio_transcription.completed";

// A bool in the connection state; the newtype keeps struct_excessive_bools off a struct that declares ten
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionFlag(pub bool);

// LiteLLM reads two provider-config methods while it holds connection state; the trait keeps BaseRealtimeConfig's associated types out of the state
pub trait RealtimeProviderConfig: Send + Sync {
    fn requires_session_configuration(&self) -> bool;
    fn unbilled_usage_on_close(&self, model: &str) -> Option<AudioTranscriptionUsage>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseAssembly {
    pub session_configuration_request: Option<String>,
    pub current_output_item_id: Option<String>,
    pub current_response_id: Option<String>,
    pub current_conversation_id: Option<String>,
    pub current_delta_chunks: Option<Vec<Json>>,
    pub current_item_chunks: Option<Vec<Json>>,
    pub current_delta_type: Option<AllDeltaType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSetup {
    pub violation_count: usize,
    pub pending_guardrail_message: Option<String>,
    pub session_created_sent_to_client: SessionFlag,
    pub turn_detection_update_sent: SessionFlag,
    pub backend_setup_complete: SessionFlag,
    pub flushing_pending_messages: SessionFlag,
    pub pending_messages_until_setup: Vec<String>,
    pub pending_messages_byte_total: usize,
    pub content_sent_after_setup: SessionFlag,
}

impl SessionSetup {
    // LiteLLM starts every flag and counter at zero, except that setup counts as complete when the provider config needs no session
    #[must_use]
    pub const fn new(backend_setup_complete: bool) -> Self {
        Self {
            violation_count: 0,
            pending_guardrail_message: None,
            session_created_sent_to_client: SessionFlag(false),
            turn_detection_update_sent: SessionFlag(false),
            backend_setup_complete: SessionFlag(backend_setup_complete),
            flushing_pending_messages: SessionFlag(false),
            pending_messages_until_setup: Vec::new(),
            pending_messages_byte_total: 0,
            content_sent_after_setup: SessionFlag(false),
        }
    }
}

pub struct RealtimeStreaming {
    pub client: Option<Box<dyn ClientWebSocket>>,
    pub backend: Option<Box<dyn RealtimeBackend>>,
    pub logging: Option<Box<dyn RealtimeLogging>>,
    pub logging_worker: Option<Box<dyn RealtimeSuccessQueue>>,
    pub provider_config: Option<Box<dyn RealtimeProviderConfig>>,
    pub event_normalizer: Option<Box<dyn RealtimeEventNormalizer>>,
    pub logged_real_time_event_types: LoggedEventTypes,
    pub model: String,
    pub user_api_key_dict: Option<Json>,
    pub request_data: IndexMap<String, Json>,
    pub backend_sent_frames: SessionFlag,
    pub messages: Vec<IndexMap<String, Json>>,
    pub input_message: IndexMap<String, Json>,
    pub input_messages: Vec<IndexMap<String, Json>>,
    pub session_tools: Vec<IndexMap<String, Json>>,
    pub tool_calls: Vec<IndexMap<String, Json>>,
    pub client_wants_beta: SessionFlag,
    pub backend_uses_beta_protocol: SessionFlag,
    pub response: ResponseAssembly,
    pub setup: SessionSetup,
    pub force_transcription_model: Option<String>,
    pub is_transcription_session: SessionFlag,
}

#[derive(Default)]
pub struct ConnectionSetup {
    pub client: Option<Box<dyn ClientWebSocket>>,
    pub backend: Option<Box<dyn RealtimeBackend>>,
    pub logging: Option<Box<dyn RealtimeLogging>>,
    pub logging_worker: Option<Box<dyn RealtimeSuccessQueue>>,
    pub provider_config: Option<Box<dyn RealtimeProviderConfig>>,
    pub event_normalizer: Option<Box<dyn RealtimeEventNormalizer>>,
    pub logged_real_time_event_types: LoggedEventTypes,
    pub model: String,
    pub user_api_key_dict: Option<Json>,
    pub request_data: Option<IndexMap<String, Json>>,
    pub force_transcription_model: Option<String>,
    pub client_wants_beta: bool,
    pub backend_uses_beta_protocol: Option<bool>,
}

impl fmt::Debug for RealtimeStreaming {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RealtimeStreaming")
            .field("model", &self.model)
            .field("messages", &self.messages)
            .field("input_messages", &self.input_messages)
            .field("session_tools", &self.session_tools)
            .field("tool_calls", &self.tool_calls)
            .field("response", &self.response)
            .field("setup", &self.setup)
            .finish_non_exhaustive()
    }
}

impl RealtimeStreaming {
    // LiteLLM stores the client socket and this connection's beta preference, model, request data and transcription mode before it relays a frame
    #[must_use]
    pub fn new(connection: ConnectionSetup) -> Self {
        Self {
            client: connection.client,
            backend: connection.backend,
            logging: connection.logging,
            logging_worker: connection.logging_worker,
            event_normalizer: connection.event_normalizer,
            model: connection.model,
            user_api_key_dict: connection.user_api_key_dict,
            request_data: connection.request_data.unwrap_or_default(),
            logged_real_time_event_types: connection.logged_real_time_event_types,
            backend_sent_frames: SessionFlag(false),
            messages: Vec::new(),
            input_message: IndexMap::new(),
            input_messages: Vec::new(),
            session_tools: Vec::new(),
            tool_calls: Vec::new(),
            client_wants_beta: SessionFlag(connection.client_wants_beta),
            backend_uses_beta_protocol: SessionFlag(
                connection
                    .backend_uses_beta_protocol
                    .unwrap_or(connection.client_wants_beta),
            ),
            response: ResponseAssembly::default(),
            setup: SessionSetup::new(
                connection
                    .provider_config
                    .as_ref()
                    .is_none_or(|config| config.requires_session_configuration()),
            ),
            is_transcription_session: SessionFlag(connection.force_transcription_model.is_some()),
            force_transcription_model: connection.force_transcription_model,
            provider_config: connection.provider_config,
        }
    }
}
