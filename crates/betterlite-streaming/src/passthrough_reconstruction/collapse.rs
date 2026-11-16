use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use serde::Serialize;

use crate::passthrough_reconstruction::error::EventError;
use crate::passthrough_reconstruction::events::{
    RawSseChunk, integer_field, object_field, split_sse_chunk_into_events, text_field,
};

// Anthropic SSE block/delta types the fast path must not merge; each keeps the legacy reconstruction.
const DISALLOWED_DELTA_TYPES: [&str; 4] = [
    "input_json_delta",
    "thinking_delta",
    "signature_delta",
    "citations_delta",
];

#[derive(Serialize)]
struct TextDeltaEvent<'text> {
    #[serde(rename = "type")]
    kind: &'text str,
    index: i64,
    delta: TextDelta<'text>,
}

#[derive(Serialize)]
struct TextDelta<'text> {
    #[serde(rename = "type")]
    kind: &'text str,
    text: &'text str,
}

#[derive(Default)]
struct Collapse {
    text_blocks: BTreeSet<Option<i64>>,
    events: Vec<String>,
    pending: Vec<String>,
    pending_index: Option<i64>,
    saw_text_delta: bool,
}

pub fn collapse_pure_text_chunks(
    chunks: &[RawSseChunk],
) -> Result<Option<Vec<String>>, EventError> {
    let mut collapse = Collapse::default();
    for chunk in chunks {
        for event in split_sse_chunk_into_events(chunk)? {
            if !collapse.visit(&event)? {
                return Ok(None);
            }
        }
    }
    collapse.flush()?;
    if !collapse.saw_text_delta {
        return Ok(None);
    }
    Ok(Some(collapse.events))
}

impl Collapse {
    // A data payload that is not a JSON object makes the stream impure; a bare event line only passes.
    fn visit(&mut self, event: &str) -> Result<bool, EventError> {
        let Some((_, payload)) = event.split_once("data:") else {
            return self.bare_event(event);
        };
        let Ok(value) = serde_json::from_str::<Json>(payload.trim()) else {
            return Ok(false);
        };
        if !matches!(value, Json::Object(_)) {
            return Ok(false);
        }
        match text_field(&value, "type") {
            Some("content_block_start") => self.block_start(&value, event),
            Some("content_block_delta") => Ok(self.text_delta(&value)),
            Some("ping") => Ok(true),
            _ => {
                self.pass_through(event)?;
                Ok(true)
            }
        }
    }

    fn flush(&mut self) -> Result<(), EventError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let text = self.pending.join("");
        let merged = TextDeltaEvent {
            kind: "content_block_delta",
            index: self.pending_index.unwrap_or(0),
            delta: TextDelta {
                kind: "text_delta",
                text: &text,
            },
        };
        let payload = serde_json::to_string(&merged)?;
        self.events.push(format!("data: {payload}"));
        self.pending.clear();
        self.pending_index = None;
        Ok(())
    }

    fn pass_through(&mut self, event: &str) -> Result<(), EventError> {
        self.flush()?;
        self.events.push(event.to_owned());
        Ok(())
    }

    // The interior markers are high-frequency no-ops; every other bare line keeps chunk order.
    fn bare_event(&mut self, event: &str) -> Result<bool, EventError> {
        let name = event.strip_prefix("event:").map_or("", str::trim);
        if matches!(name, "content_block_delta" | "ping") {
            return Ok(true);
        }
        self.pass_through(event)?;
        Ok(true)
    }

    fn block_start(&mut self, data: &Json, event: &str) -> Result<bool, EventError> {
        let block = object_field(data, "content_block");
        if block.and_then(|fields| text_field(fields, "type")) != Some("text") {
            return Ok(false);
        }
        self.text_blocks.insert(integer_field(data, "index"));
        self.pass_through(event)?;
        Ok(true)
    }

    fn text_delta(&mut self, data: &Json) -> bool {
        let delta = object_field(data, "delta");
        let kind = delta.and_then(|fields| text_field(fields, "type"));
        if kind.is_some_and(|delta_type| DISALLOWED_DELTA_TYPES.contains(&delta_type)) {
            return false;
        }
        if kind != Some("text_delta") {
            return false;
        }
        let index = integer_field(data, "index");
        if !self.text_blocks.contains(&index) {
            return false;
        }
        // pending_index is set only together with pending text, so LiteLLM's pending_text guard is implied.
        if self.pending_index.is_some() && index != self.pending_index {
            return false;
        }
        self.saw_text_delta = true;
        self.pending_index = index;
        let text = delta.and_then(|fields| text_field(fields, "text"));
        self.pending.push(text.unwrap_or_default().to_owned());
        true
    }
}
