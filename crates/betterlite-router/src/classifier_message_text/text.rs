use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ContentPart, MessageContent};

use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::MarkerPair;

#[must_use]
pub fn message_text(content: Option<&MessageContent>) -> Result<String, ClassifierTextError> {
    match content {
        None => Ok(String::new()),
        Some(MessageContent::Text(text)) => Ok(text.clone()),
        Some(MessageContent::Parts(parts)) => {
            let mut texts: Vec<String> = Vec::with_capacity(parts.len());
            for (index, part) in parts.iter().enumerate() {
                if part.kind != "text" {
                    continue;
                }
                let text = part.fields.get("text").map_or(Ok(String::new()), |value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(ClassifierTextError::PartTextNotString { part: index })
                })?;
                texts.push(text);
            }
            Ok(texts.join(" ").trim().to_owned())
        }
    }
}

#[must_use]
pub fn json_message_text(content: Option<&Json>) -> Result<String, ClassifierTextError> {
    let Some(content) = content else {
        return Ok(String::new());
    };
    match content {
        Json::Text(text) => Ok(text.clone()),
        Json::List(parts) => {
            let mut texts: Vec<String> = Vec::with_capacity(parts.len());
            for (index, part) in parts.iter().enumerate() {
                let Json::Object(fields) = part else {
                    continue;
                };
                if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "text") {
                    continue;
                }
                let text = match fields.get("text") {
                    None => String::new(),
                    Some(Json::Text(text)) => text.clone(),
                    Some(_) => {
                        return Err(ClassifierTextError::PartTextNotString { part: index });
                    }
                };
                texts.push(text);
            }
            Ok(texts.join(" ").trim().to_owned())
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(String::new()),
    }
}

#[must_use]
pub fn human_text(
    content: Option<&MessageContent>,
    markers: &[MarkerPair<'_>],
) -> Result<String, ClassifierTextError> {
    let text = message_text(content)?;
    Ok(strip_reminder_blocks(&text, markers))
}

#[must_use]
pub fn json_human_text(
    content: Option<&Json>,
    markers: &[MarkerPair<'_>],
) -> Result<String, ClassifierTextError> {
    let text = json_message_text(content)?;
    Ok(strip_reminder_blocks(&text, markers))
}

#[must_use]
pub fn strip_reminder_blocks(text: &str, markers: &[MarkerPair<'_>]) -> String {
    let lowered: Vec<char> = text.to_lowercase().chars().collect();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for marker in markers {
        let open: Vec<char> = marker.open.chars().collect();
        let close: Vec<char> = marker.close.chars().collect();
        let mut cursor = 0_usize;
        while let Some(start) = find_chars(&lowered, &open, cursor) {
            let Some(end) = find_chars(&lowered, &close, start.saturating_add(open.len())) else {
                break;
            };
            cursor = end.saturating_add(close.len());
            spans.push((start, cursor));
        }
    }
    if spans.is_empty() {
        return text.trim().to_owned();
    }
    spans.sort_unstable();
    let original: Vec<char> = text.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut kept_from = 0_usize;
    for (start, end) in spans {
        if start > kept_from {
            push_trimmed(&mut parts, &original, kept_from, start);
        }
        kept_from = kept_from.max(end);
    }
    push_trimmed(&mut parts, &original, kept_from, original.len());
    parts.join(" ")
}

// literal char search, not a regex: an unclosed open marker only ends the scan, so the walk stays linear
fn find_chars(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let last = haystack.len().checked_sub(needle.len())?;
    if from > last {
        return None;
    }
    (from..=last).find(|index| {
        haystack
            .iter()
            .skip(*index)
            .take(needle.len())
            .eq(needle.iter().copied())
    })
}

fn push_trimmed(parts: &mut Vec<String>, original: &[char], from: usize, to: usize) {
    let end = to.min(original.len());
    let start = from.min(end);
    let chunk: String = original
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect();
    let trimmed = chunk.trim();
    if !trimmed.is_empty() {
        parts.push(trimmed.to_owned());
    }
}
