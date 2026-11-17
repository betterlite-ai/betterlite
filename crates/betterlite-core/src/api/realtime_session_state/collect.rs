use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::RealtimeStreaming;
use super::records::event_type;

fn message_record(role: &str, content: &str) -> IndexMap<String, Json> {
    IndexMap::from([
        ("role".to_owned(), Json::Text(role.to_owned())),
        ("content".to_owned(), Json::Text(content.to_owned())),
    ])
}

pub(crate) fn push_text_message(
    messages: &mut Vec<IndexMap<String, Json>>,
    role: &str,
    text: &str,
) {
    if !text.is_empty() {
        messages.push(message_record(role, text));
    }
}

pub(crate) fn collect_record_input(state: &mut RealtimeStreaming, record: &IndexMap<String, Json>) {
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
    mark_transcription_session(state, session);
}

pub(crate) fn mark_transcription_session(
    state: &mut RealtimeStreaming,
    session: &IndexMap<String, Json>,
) {
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
