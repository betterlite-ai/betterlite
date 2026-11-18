use betterlite_models::decisions::Json;

use super::super::realtime_contracts::events::AllDeltaType;

// A bool in the connection state; the newtype keeps struct_excessive_bools off a struct that declares ten
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionFlag(pub bool);

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
