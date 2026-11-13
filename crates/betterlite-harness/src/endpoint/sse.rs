use crate::types::JsonObject;

use super::usage::{child_object, usage_from_mapping};

#[derive(Debug, Default)]
pub struct SSEUsageParser {
    pub input_tokens: u64,
    pub output_tokens: u64,
    buffer: Vec<u8>,
}

impl SSEUsageParser {
    pub fn feed(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        let Some(position) = self.buffer.iter().rposition(|byte| *byte == b'\n') else {
            return;
        };
        let complete = position.saturating_add(1);
        let tail = self.buffer.split_off(complete);
        let head = std::mem::replace(&mut self.buffer, tail);
        for line in head.split(|byte| *byte == b'\n') {
            self.feed_line(line);
        }
    }

    pub fn close(&mut self) {
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.feed_line(&line);
        }
    }

    fn feed_line(&mut self, line: &[u8]) {
        let text = line.trim_ascii();
        let Some(payload) = text.strip_prefix(b"data:") else {
            return;
        };
        let payload = payload.trim_ascii();
        if payload.is_empty() || payload == b"[DONE]" {
            return;
        }
        let Ok(event) = serde_json::from_slice::<JsonObject>(payload) else {
            return;
        };
        self.absorb(&event);
    }

    fn absorb(&mut self, event: &JsonObject) {
        match event.get("type").and_then(|value| value.as_str()) {
            Some("message_start") => {
                if let Some(message) = child_object(event, "message") {
                    self.set(usage_of(&message));
                }
            }
            Some("message_delta") => self.set(usage_of(event)),
            Some("response.completed") => {
                if let Some(response) = child_object(event, "response") {
                    self.set(usage_of(&response));
                }
            }
            Some(_) | None => {
                if event.get("usage").is_some_and(|value| value.is_object()) {
                    self.set(usage_of(event));
                }
            }
        }
    }

    // message_delta output_tokens is the running total, so a later event replaces the earlier count
    fn set(&mut self, tokens: (u64, u64)) {
        if tokens.0 != 0 {
            self.input_tokens = tokens.0;
        }
        if tokens.1 != 0 {
            self.output_tokens = tokens.1;
        }
    }
}

fn usage_of(event: &JsonObject) -> (u64, u64) {
    let Some(usage) = child_object(event, "usage") else {
        return (0, 0);
    };
    usage_from_mapping(&usage)
}
