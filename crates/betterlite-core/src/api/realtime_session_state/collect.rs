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

impl RealtimeStreaming {
    // LiteLLM ignores a client event it cannot read: malformed JSON, a non-object item and a content entry of the wrong type all leave the log alone
    pub fn collect_client_input(&mut self, message: SessionInput<'_>) {
        match message {
            SessionInput::Record(record) => self.collect_record_input(record),
            SessionInput::Text(text) => {
                let Ok(record) = decoded_record(text.as_bytes()) else {
                    return;
                };
                self.collect_record_input(&record);
            }
        }
    }

    pub fn collect_backend_input(&mut self, event: &IndexMap<String, Json>) {
        let Some(Json::Text(kind)) = event.get("type") else {
            return;
        };
        if kind != TRANSCRIPTION_COMPLETED {
            return;
        }
        let Some(Json::Text(transcript)) = event.get("transcript") else {
            return;
        };
        push_text_message(&mut self.input_messages, "user", transcript);
    }

    pub fn detect_transcription_session(&mut self, event: &IndexMap<String, Json>) {
        match event_type(event) {
            Some("transcription_session.created" | "transcription_session.updated") => {
                self.is_transcription_session.0 = true;
            }
            Some("session.created" | "session.updated") => {
                let Some(Json::Object(session)) = event.get("session") else {
                    return;
                };
                if matches!(session.get("type"), Some(Json::Text(kind)) if kind == "transcription")
                {
                    self.is_transcription_session.0 = true;
                }
            }
            _ => {}
        }
    }

    fn collect_record_input(&mut self, record: &IndexMap<String, Json>) {
        match event_type(record) {
            Some("conversation.item.create") => self.collect_conversation_item(record),
            Some("session.update") => self.collect_session_update(record),
            _ => {}
        }
    }

    fn collect_conversation_item(&mut self, event: &IndexMap<String, Json>) {
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
                push_text_message(&mut self.input_messages, "user", text);
            }
        }
    }

    fn collect_session_update(&mut self, event: &IndexMap<String, Json>) {
        let Some(Json::Object(session)) = event.get("session") else {
            return;
        };
        if let Some(Json::Text(instructions)) = session.get("instructions") {
            push_text_message(&mut self.input_messages, "system", instructions);
        }
        if let Some(Json::List(tools)) = session.get("tools") {
            self.replace_session_tools(tools);
        }
        if matches!(session.get("type"), Some(Json::Text(kind)) if kind == "transcription") {
            self.is_transcription_session.0 = true;
        }
    }

    // an empty list leaves the previous tools set, and so does a list holding an entry that is not an object
    fn replace_session_tools(&mut self, tools: &[Json]) {
        let mut records = Vec::with_capacity(tools.len());
        for tool in tools {
            let Json::Object(record) = tool else {
                return;
            };
            records.push(record.clone());
        }
        if !records.is_empty() {
            self.session_tools = records;
        }
    }
}
