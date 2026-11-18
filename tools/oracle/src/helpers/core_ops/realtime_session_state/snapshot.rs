use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_core::api::realtime_session_state::{DEFAULT_LOGGED_EVENT_TYPES, RealtimeStreaming};
use serde_json::{Value, json};

pub(crate) fn snapshot(state: &RealtimeStreaming) -> Value {
    json!({
        "logged_event_types": selection_value(&state.logged_real_time_event_types),
        "model": state.model,
        "user_api_key_dict": state.user_api_key_dict,
        "request_data": state.request_data,
        "client_wants_beta": state.client_wants_beta.0,
        "backend_uses_beta_protocol": state.backend_uses_beta_protocol.0,
        "backend_sent_frames": state.backend_sent_frames.0,
        "has_provider_config": state.provider_config.is_some(),
        "force_transcription_model": state.force_transcription_model,
        "is_transcription_session": state.is_transcription_session.0,
        "messages": state.messages,
        "input_message": state.input_message,
        "input_messages": state.input_messages,
        "session_tools": state.session_tools,
        "tool_calls": state.tool_calls,
        "response": {
            "current_output_item_id": state.response.current_output_item_id,
            "current_response_id": state.response.current_response_id,
            "current_conversation_id": state.response.current_conversation_id,
            "current_delta_chunks": state.response.current_delta_chunks,
            "current_item_chunks": state.response.current_item_chunks,
            "current_delta_type": state.response.current_delta_type,
            "session_configuration_request": state.response.session_configuration_request,
        },
        "setup": {
            "violation_count": state.setup.violation_count,
            "pending_guardrail_message": state.setup.pending_guardrail_message,
            "session_created_sent_to_client": state.setup.session_created_sent_to_client.0,
            "turn_detection_update_sent": state.setup.turn_detection_update_sent.0,
            "backend_setup_complete": state.setup.backend_setup_complete.0,
            "flushing_pending_messages": state.setup.flushing_pending_messages.0,
            "pending_messages_until_setup": state.setup.pending_messages_until_setup,
            "pending_messages_byte_total": state.setup.pending_messages_byte_total,
            "content_sent_after_setup": state.setup.content_sent_after_setup.0,
        },
    })
}

fn selection_value(types: &LoggedEventTypes) -> Value {
    match types {
        LoggedEventTypes::Default => json!(DEFAULT_LOGGED_EVENT_TYPES),
        LoggedEventTypes::All => json!("*"),
        LoggedEventTypes::Only(names) => json!(names),
    }
}
