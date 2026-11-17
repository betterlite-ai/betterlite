use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::RealtimeStreaming;
use super::TRANSCRIPTION_COMPLETED;
use super::logging::SessionInput;
use super::records::{decoded_record, event_type};

fn message_record(role: &str, content: &str) -> IndexMap<String, Json> {
    IndexMap::from([
        ("role".to_owned(), Json::Text(role.to_owned())),
        ("content".to_owned(), Json::Text(content.to_owned())),
    ])
}

fn push_text_message(messages: &mut Vec<IndexMap<String, Json>>, role: &str, text: &str) {
    if !text.is_empty() {
        messages.push(message_record(role, text));
    }
}

// LiteLLM ignores a client event it cannot read: malformed JSON, a non-object item and a content entry of the wrong type all leave the log alone
pub(crate) fn collect_client_input(state: &mut RealtimeStreaming, message: SessionInput<'_>) {
    match message {
        SessionInput::Record(record) => collect_record_input(state, record),
        SessionInput::Text(text) => {
            let Ok(record) = decoded_record(text.as_bytes()) else {
                return;
            };
            collect_record_input(state, &record);
        }
    }
}

pub(crate) fn collect_backend_input(state: &mut RealtimeStreaming, event: &IndexMap<String, Json>) {
    let Some(Json::Text(kind)) = event.get("type") else {
        return;
    };
    if kind != TRANSCRIPTION_COMPLETED {
        return;
    }
    let Some(Json::Text(transcript)) = event.get("transcript") else {
        return;
    };
    push_text_message(&mut state.input_messages, "user", transcript);
}

pub(crate) fn detect_transcription_session(
    state: &mut RealtimeStreaming,
    event: &IndexMap<String, Json>,
) {
    match event_type(event) {
        Some("transcription_session.created" | "transcription_session.updated") => {
            state.is_transcription_session.0 = true;
        }
        Some("session.created" | "session.updated") => {
            let Some(Json::Object(session)) = event.get("session") else {
                return;
            };
            if matches!(session.get("type"), Some(Json::Text(kind)) if kind == "transcription") {
                state.is_transcription_session.0 = true;
            }
        }
        _ => {}
    }
}

fn collect_record_input(state: &mut RealtimeStreaming, record: &IndexMap<String, Json>) {
    match event_type(record) {
        Some("conversation.item.create") => collect_conversation_item(state, record),
        Some("session.update") => collect_session_update(state, record),
        _ => {}
    }
}

fn collect_conversation_item(state: &mut RealtimeStreaming, event: &IndexMap<String, Json>) {
    let Some(Json::Object(item)) = event.get("item") else {
        return;
    };
    if !matches!(item.get("role"), Some(Json::Text(role)) if role == "user") {
        return;
    }
    let Some(Json::List(contents)) = item.get("content") else {
        return;
    };
    for content in contents {
        let Json::Object(entry) = content else {
            continue;
        };
        if !matches!(entry.get("type"), Some(Json::Text(kind)) if kind == "input_text") {
            continue;
        }
        if let Some(Json::Text(text)) = entry.get("text") {
            push_text_message(&mut state.input_messages, "user", text);
        }
    }
}

fn collect_session_update(state: &mut RealtimeStreaming, event: &IndexMap<String, Json>) {
    let Some(Json::Object(session)) = event.get("session") else {
        return;
    };
    if let Some(Json::Text(instructions)) = session.get("instructions") {
        push_text_message(&mut state.input_messages, "system", instructions);
    }
    if let Some(Json::List(tools)) = session.get("tools") {
        replace_session_tools(state, tools);
    }
    tracing::debug!("Realtime session.type: {:?}", session.get("type"));
    if matches!(session.get("type"), Some(Json::Text(kind)) if kind == "transcription") {
        state.is_transcription_session.0 = true;
    }
}

// an empty list leaves the previous tools set, and so does a list holding an entry that is not an object
fn replace_session_tools(state: &mut RealtimeStreaming, tools: &[Json]) {
    let mut records = Vec::with_capacity(tools.len());
    for tool in tools {
        let Json::Object(record) = tool else {
            return;
        };
        records.push(record.clone());
    }
    if !records.is_empty() {
        state.session_tools = records;
    }
}
