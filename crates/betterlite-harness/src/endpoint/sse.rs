mod tests;

use std::mem;

use super::usage::{Lenient, Payload, usage_from_mapping};

// A response.completed event carries the whole response, base64 image output included, so a smaller cap drops real usage.
const MAX_PENDING_LINE: usize = 0x200_0000;

#[derive(Debug, Default)]
pub struct SSEUsageParser {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub buffer: Vec<u8>,
    pub skipping: bool,
}

impl SSEUsageParser {
    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
            let (line, after) = rest.split_at(end);
            self.finish_line(line);
            rest = after.get(1..).unwrap_or_default();
        }
        self.hold(rest);
    }

    // The bytes after the last newline wait for the next chunk, and an oversized line is dropped up to its newline.
    fn hold(&mut self, partial: &[u8]) {
        if self.skipping {
            return;
        }
        if self.buffer.len().saturating_add(partial.len()) > MAX_PENDING_LINE {
            self.buffer = Vec::new();
            self.skipping = true;
        } else {
            self.buffer.extend_from_slice(partial);
        }
    }

    fn finish_line(&mut self, tail: &[u8]) {
        if self.skipping {
            self.skipping = false;
        } else if self.buffer.len().saturating_add(tail.len()) <= MAX_PENDING_LINE {
            let mut line = mem::take(&mut self.buffer);
            line.extend_from_slice(tail);
            self.feed_line(&line);
        } else {
            self.buffer = Vec::new();
        }
    }

    pub fn close(&mut self) {
        if !self.buffer.is_empty() {
            let line = mem::take(&mut self.buffer);
            self.feed_line(&line);
        }
    }

    fn feed_line(&mut self, line: &[u8]) {
        let text = line.trim_ascii();
        let Some(data) = text.strip_prefix(b"data:") else {
            return;
        };
        let payload = data.trim_ascii();
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
        let selected = match kind {
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
        if let Some(counts) = selected.and_then(Lenient::value) {
            self.set(usage_from_mapping(counts));
        }
    }

    // message_delta output_tokens is the running total, so a later event replaces the earlier count
    const fn set(&mut self, tokens: (u64, u64)) {
        if tokens.0 != 0 {
            self.input_tokens = tokens.0;
        }
        if tokens.1 != 0 {
            self.output_tokens = tokens.1;
        }
    }
}
