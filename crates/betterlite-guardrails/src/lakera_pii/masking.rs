use serde_json::value::RawValue;
use std::cmp::Reverse;
use std::collections::BTreeMap;

use super::error::LakeraPiiError;
use super::wire::{LakeraResponse, Message, PayloadItem};

// Replaces each detected PII span in messages with a [MASKED <TYPE>] token, counting detections by type; returns the messages unchanged when response carries no payload at all.
#[must_use]
pub fn mask_pii_in_messages(
    messages: &[Message],
    response: Option<&LakeraResponse>,
    counts: &mut BTreeMap<String, u64>,
) -> Vec<Message> {
    let Some(payload) = response
        .and_then(|value| value.payload.as_ref())
        .filter(|items| !items.is_empty())
    else {
        return messages.to_vec();
    };
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| mask_message(index, message, payload, counts))
        .collect()
}

fn mask_message(
    index: usize,
    message: &Message,
    payload: &[PayloadItem],
    counts: &mut BTreeMap<String, u64>,
) -> Message {
    let Some(content) = message
        .content
        .as_ref()
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
        .filter(|content| !content.is_empty())
    else {
        return message.clone();
    };
    let message_id = u64::try_from(index).unwrap_or(u64::MAX);
    let mut modifications: Vec<&PayloadItem> = payload
        .iter()
        .filter(|item| item.message_id == Some(message_id))
        .collect();
    if modifications.is_empty() {
        return message.clone();
    }
    modifications.sort_by_key(|item| Reverse((item.start.unwrap_or(0), item.end.unwrap_or(0))));
    let mut masked = content;
    for item in modifications {
        apply_modification(&mut masked, item, counts);
    }
    let mut updated = message.clone();
    if let Ok(serialized) = serde_json::to_string(&masked)
        && let Ok(raw) = RawValue::from_string(serialized)
    {
        updated.content = Some(raw);
    }
    updated
}

// Masks are applied back to front so each earlier span's character offsets stay valid once a later span has already been replaced.
fn apply_modification(masked: &mut String, item: &PayloadItem, counts: &mut BTreeMap<String, u64>) {
    let Some(detector_type) = item
        .detector_type
        .as_deref()
        .filter(|kind| !kind.is_empty())
    else {
        return;
    };
    if item.extra.contains_key("start") || item.extra.contains_key("end") {
        return;
    }
    let start = item.start.unwrap_or_default();
    let end = item.end.unwrap_or_default();
    let label = detector_label(detector_type);
    if let (Some(start_byte), Some(end_byte)) = (
        character_byte_index(masked, start),
        character_byte_index(masked, end),
    ) && start_byte < end_byte
    {
        masked.replace_range(start_byte..end_byte, &format!("[MASKED {label}]"));
    }
    counts
        .entry(label)
        .and_modify(|count| *count = count.saturating_add(1))
        .or_insert(1);
}

// detector_type is a slash-separated path such as pii/email; the mask label is its last, upper-cased segment, falling back to PII when that segment is empty (for example a trailing-slash detector type).
fn detector_label(detector_type: &str) -> String {
    let segment = detector_type.rsplit('/').next().unwrap_or("");
    if segment.is_empty() {
        "PII".to_owned()
    } else {
        segment.to_uppercase()
    }
}

fn character_byte_index(text: &str, character_index: u64) -> Option<usize> {
    let character_offset = usize::try_from(character_index).ok()?;
    if let Some((byte_index, _)) = text.char_indices().nth(character_offset) {
        Some(byte_index)
    } else if text.chars().count() == character_offset {
        Some(text.len())
    } else {
        None
    }
}

// Returns true when response.breakdown reports at least one violation and every detected violation is a pii/* detector, so masking alone is safe.
#[must_use]
pub fn is_only_pii_violation(response: Option<&LakeraResponse>) -> bool {
    let Some(breakdown) = response.and_then(|value| value.breakdown.as_ref()) else {
        return false;
    };
    let mut has_violations = false;
    for item in breakdown {
        if item.detected != Some(true) {
            continue;
        }
        has_violations = true;
        if !item
            .detector_type
            .as_deref()
            .is_some_and(|kind| kind.starts_with("pii/"))
        {
            return false;
        }
    }
    has_violations
}

#[must_use]
pub fn blocked_error(response: Option<&LakeraResponse>) -> LakeraPiiError {
    let Some(value) = response else {
        return LakeraPiiError::Blocked { response: None };
    };
    LakeraPiiError::Blocked {
        response: Some(value.clone()),
    }
}
