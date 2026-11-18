pub mod collect;
pub mod error;
pub mod logging;
pub mod records;
pub mod state;
pub mod store;
mod tests;
pub mod usage;

use std::fmt;

use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::realtime_contracts::events::AudioTranscriptionUsage;
use super::realtime_contracts::transport::{
    ClientWebSocket, RealtimeBackend, RealtimeEventNormalizer,
};
use error::SessionStateError;
use logging::{RealtimeLogging, RealtimeSuccessQueue, SessionInput, record_details};
use state::{ResponseAssembly, SessionFlag, SessionSetup};
use store::SessionMessage;
use usage::usage_record;

pub const DEFAULT_LOGGED_EVENT_TYPES: [&str; 5] = [
    "session.created",
    "response.create",
    "response.done",
    "conversation.item.added",
    "conversation.item.done",
];
pub const TRANSCRIPTION_COMPLETED: &str = "conversation.item.input_audio_transcription.completed";

// LiteLLM reads two provider-config methods while it holds connection state; the trait keeps BaseRealtimeConfig's associated types out of the state
pub trait RealtimeProviderConfig: Send + Sync {
    fn requires_session_configuration(&self) -> bool;
    fn unbilled_usage_on_close(&self, model: &str) -> Option<AudioTranscriptionUsage>;
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealtimeStreaming")
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

    // LiteLLM collects response.done tool calls before it applies the event filter, so an unlogged response still bills its functions
    pub fn store_message(&mut self, message: SessionMessage<'_>) -> Result<(), SessionStateError> {
        let record = store::decoded_message(message)?;
        self.collect_response_tool_calls(&record);
        if self.should_store_message(&record) {
            if let Err(error) = records::check_session_event(&record) {
                tracing::debug!("Error parsing message for logging: {error}");
            }
            self.messages.push(record);
        }
        Ok(())
    }

    #[must_use]
    pub fn should_store_message(&self, record: &IndexMap<String, Json>) -> bool {
        match &self.logged_real_time_event_types {
            LoggedEventTypes::All => true,
            LoggedEventTypes::Default => records::event_type(record)
                .is_some_and(|event| DEFAULT_LOGGED_EVENT_TYPES.contains(&event)),
            LoggedEventTypes::Only(types) => records::event_type(record)
                .is_some_and(|event| types.iter().any(|logged| logged == event)),
        }
    }

    pub fn collect_response_tool_calls(&mut self, event: &IndexMap<String, Json>) {
        if records::event_type(event) != Some("response.done") {
            return;
        }
        let Some(Json::Object(response)) = event.get("response") else {
            return;
        };
        let Some(Json::List(output)) = response.get("output") else {
            return;
        };
        for item in output {
            let Json::Object(entry) = item else {
                // LiteLLM reads item.get on every entry, and a non-object entry ends its loop
                break;
            };
            if matches!(entry.get("type"), Some(Json::Text(kind)) if kind == "function_call") {
                self.tool_calls.push(records::tool_call_record(entry));
            }
        }
    }

    // LiteLLM ignores a client event it cannot read: malformed JSON, a non-object item and a content entry of the wrong type all leave the log alone
    pub fn collect_client_input(&mut self, message: SessionInput<'_>) {
        match message {
            SessionInput::Record(record) => collect::collect_record_input(self, record),
            SessionInput::Text(text) => {
                let Ok(record) = records::decoded_record(text.as_bytes()) else {
                    return;
                };
                collect::collect_record_input(self, &record);
            }
        }
    }

    pub fn collect_backend_input(&mut self, event: &IndexMap<String, Json>) {
        if records::event_type(event) != Some(TRANSCRIPTION_COMPLETED) {
            return;
        }
        let Some(Json::Text(transcript)) = event.get("transcript") else {
            return;
        };
        collect::push_text_message(&mut self.input_messages, "user", transcript);
    }

    pub fn detect_transcription_session(&mut self, event: &IndexMap<String, Json>) {
        match records::event_type(event) {
            Some("transcription_session.created" | "transcription_session.updated") => {
                self.is_transcription_session.0 = true;
            }
            Some("session.created" | "session.updated") => {
                let Some(Json::Object(session)) = event.get("session") else {
                    return;
                };
                collect::mark_transcription_session(self, session);
            }
            _ => {}
        }
    }

    pub fn store_input(&mut self, message: SessionInput<'_>) {
        self.input_message = logging::retained_input(message);
        self.collect_client_input(message);
        if let Some(logging) = self.logging.as_deref() {
            logging.pre_call(&message, "");
        }
    }

    #[must_use]
    pub fn log_messages(&self) -> bool {
        let Some(logging) = self.logging.as_deref() else {
            return false;
        };
        record_details(logging, self);
        let work = logging.dispatch_success_handlers(&self.messages);
        let queue = self.logging_worker.as_deref();
        if !queue.is_some_and(|sender| sender.enqueue(work)) {
            return false;
        }
        logging.record_model_call_detail(logging::SUCCESS_LOGGED_KEY, Json::Bool(true));
        true
    }

    // Only the type and usage are kept: the transcript already sits in input_messages, and an event the filter logs needs no second copy
    pub fn capture_transcription_usage(&mut self, event: &IndexMap<String, Json>) {
        let Some(usage) = event.get("usage") else {
            return;
        };
        if matches!(usage, Json::Null) || self.should_store_message(event) {
            return;
        }
        self.messages.push(usage_record(usage.clone()));
    }

    pub fn flush_unbilled_usage(&mut self) -> Result<(), SessionStateError> {
        let Some(config) = self.provider_config.as_deref() else {
            return Ok(());
        };
        let Some(usage) = config.unbilled_usage_on_close(&self.model) else {
            return Ok(());
        };
        let encoded = serde_json::from_value(serde_json::to_value(usage)?)?;
        let event = usage_record(encoded);
        self.store_message(SessionMessage::Record(&event))?;
        self.capture_transcription_usage(&event);
        Ok(())
    }
}
