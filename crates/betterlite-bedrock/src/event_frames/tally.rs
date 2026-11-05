use std::collections::BTreeMap;

use betterlite_redact::secret::Redactor;

use super::AwsEvent;
use super::error::EventFrameError;

const STREAM_HEAD_BYTES: usize = 200;
const NO_EVENTS_STATUS: u16 = 502;

// Mirrors _EventStreamTally: counts what a Bedrock stream delivered so a silent or cut body becomes an error.
#[derive(Debug, Default)]
pub struct EventStreamTally {
    bytes_received: u64,
    bytes_decoded: u64,
    events: u64,
    head: Vec<u8>,
}

impl EventStreamTally {
    pub fn add_chunk(&mut self, chunk: &[u8]) {
        self.bytes_received = self
            .bytes_received
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        let room = STREAM_HEAD_BYTES.saturating_sub(self.head.len());
        self.head.extend(chunk.iter().take(room));
    }

    pub fn add_event(&mut self, event: &AwsEvent) {
        self.events = self.events.saturating_add(1);
        self.bytes_decoded = self
            .bytes_decoded
            .saturating_add(u64::from(event.total_length));
    }

    #[must_use]
    // The first bytes and header values come from an unknown upstream, so the message goes through the redactor.
    pub fn undecoded_stream_error(
        &self,
        response_headers: Option<&BTreeMap<String, String>>,
        redactor: &Redactor,
    ) -> Option<EventFrameError> {
        let undecoded = self.bytes_received.saturating_sub(self.bytes_decoded);
        if self.events != 0 && undecoded == 0 {
            return None;
        }
        let detail = format!(
            "content-type={}, x-amzn-requestid={}, {} bytes received",
            repr_text(response_headers.and_then(|map| map.get("content-type"))),
            repr_text(response_headers.and_then(|map| map.get("x-amzn-requestid"))),
            self.bytes_received,
        );
        let message = if self.events == 0 {
            format!(
                "Bedrock answered the stream with HTTP 200 but its body decoded to no events ({detail}, first bytes={})",
                repr_bytes(&self.head),
            )
        } else {
            format!(
                "Bedrock stream ended with {undecoded} undecoded bytes after {} events ({detail})",
                self.events,
            )
        };
        Some(EventFrameError::upstream(
            NO_EVENTS_STATUS,
            redactor.redact_string(&message),
        ))
    }
}

fn repr_text(value: Option<&String>) -> String {
    let Some(text) = value else {
        return "None".to_owned();
    };
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for ch in text.chars() {
        let piece = match ch {
            '\\' => "\\\\".to_owned(),
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            _ if ch == quote => format!("\\{ch}"),
            _ if ch.is_control() => format!("\\x{:02x}", u32::from(ch)),
            _ => ch.to_string(),
        };
        out.push_str(&piece);
    }
    out.push(quote);
    out
}

fn repr_bytes(bytes: &[u8]) -> String {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut out = String::from("b");
    out.push(char::from(quote));
    for &byte in bytes {
        let piece = match byte {
            b'\\' => "\\\\".to_owned(),
            b'\n' => "\\n".to_owned(),
            b'\r' => "\\r".to_owned(),
            b'\t' => "\\t".to_owned(),
            _ if byte == quote => format!("\\{}", char::from(byte)),
            0x20..=0x7e => char::from(byte).to_string(),
            _ => format!("\\x{byte:02x}"),
        };
        out.push_str(&piece);
    }
    out.push(char::from(quote));
    out
}
