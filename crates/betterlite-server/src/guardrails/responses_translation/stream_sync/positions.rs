use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_field;
use serde_json::Value;

use crate::guardrails::responses_translation::stream_scan::{
    OUTPUT_TEXT_EVENT_TYPES, event_item, item_text,
};

use super::error::StreamSyncError;

const SPREAD_REASON: &str = "the scanned text events are not all output_text deltas with an integer output_index and content_index, so the text rewrite has nowhere to land";

#[derive(Clone, PartialEq)]
pub(super) struct TextPosition {
    output_index: Option<GuardrailValue>,
    content_index: Option<GuardrailValue>,
}

fn text_position(item: &StreamItem) -> TextPosition {
    TextPosition {
        output_index: stream_item_field(item, "output_index").cloned(),
        content_index: stream_item_field(item, "content_index").cloned(),
    }
}

fn unsigned(value: Option<&GuardrailValue>) -> Option<usize> {
    match value {
        Some(GuardrailValue::SignedInteger(number)) => usize::try_from(*number).ok(),
        Some(GuardrailValue::UnsignedInteger(number)) => usize::try_from(*number).ok(),
        Some(_) | None => None,
    }
}

fn spread_undeliverable(guardrail_name: &str) -> StreamSyncError {
    StreamSyncError::Undeliverable {
        guardrail_name: guardrail_name.to_owned(),
        reason: SPREAD_REASON.to_owned(),
    }
}

pub(super) fn scanned_text_positions(
    events: &[Value],
    guardrail_name: &str,
) -> Result<Vec<TextPosition>, StreamSyncError> {
    let mut positions: Vec<TextPosition> = Vec::new();
    for event in events {
        let Some(item) = event_item(event) else {
            continue;
        };
        if item_text(&item, "text").is_none() && item_text(&item, "delta").is_none() {
            continue;
        }
        let event_type = item_text(&item, "type");
        if !event_type.is_some_and(|kind| OUTPUT_TEXT_EVENT_TYPES.contains(&kind)) {
            return Err(spread_undeliverable(guardrail_name));
        }
        let position = text_position(&item);
        if !positions.contains(&position) {
            positions.push(position);
        }
    }
    Ok(positions)
}

pub(super) fn placeable_positions(
    positions: &[TextPosition],
    guardrail_name: &str,
) -> Result<Vec<(usize, usize)>, StreamSyncError> {
    let mut placeable = Vec::with_capacity(positions.len());
    for position in positions {
        let output = unsigned(position.output_index.as_ref());
        let content = unsigned(position.content_index.as_ref());
        let (Some(output_index), Some(content_index)) = (output, content) else {
            return Err(spread_undeliverable(guardrail_name));
        };
        placeable.push((output_index, content_index));
    }
    Ok(placeable)
}
