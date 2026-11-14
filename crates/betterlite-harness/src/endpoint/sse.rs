use super::usage::{Lenient, Payload, usage_from_mapping};

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
        let Ok(event) = serde_json::from_slice::<Payload>(payload) else {
            return;
        };
        self.absorb(&event);
    }

    // message_delta carries its usage at the top level like any other event, so it takes the last arm.
    fn absorb(&mut self, event: &Payload) {
        let kind = event
            .kind
            .as_ref()
            .and_then(Lenient::value)
            .map_or("", String::as_str);
        let counts = match kind {
            "message_start" => event
                .message
                .as_ref()
                .and_then(Lenient::value)
                .and_then(|message| message.usage.as_ref()),
            "response.completed" => event
                .response
                .as_ref()
                .and_then(Lenient::value)
                .and_then(|response| response.usage.as_ref()),
            _ => event.usage.as_ref(),
        };
        if let Some(counts) = counts.and_then(Lenient::value) {
            self.set(usage_from_mapping(counts));
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
