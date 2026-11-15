use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ContentPart, MessageContent};

use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::MarkerPair;

// a content value's text: a plain string is kept as sent, a multi-part list joins its text parts
pub enum ContentText<'content> {
    Plain(&'content str),
    Parts(Vec<PartText<'content>>),
}

// a non-text part is skipped; a text part's missing field is empty and a non-string one names its index
pub enum PartText<'part> {
    NotText,
    Missing,
    Text(&'part str),
    NotString,
}

pub trait ClassifierContent {
    fn content_text(&self) -> ContentText<'_>;
}

impl ClassifierContent for MessageContent {
    fn content_text(&self) -> ContentText<'_> {
        match self {
            Self::Text(text) => ContentText::Plain(text),
            Self::Parts(parts) => ContentText::Parts(parts.iter().map(typed_part).collect()),
        }
    }
}

impl ClassifierContent for Json {
    fn content_text(&self) -> ContentText<'_> {
        match self {
            Self::Text(text) => ContentText::Plain(text),
            Self::List(parts) => ContentText::Parts(parts.iter().map(json_part).collect()),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Object(_) => {
                ContentText::Plain("")
            }
        }
    }
}

fn typed_part(part: &ContentPart) -> PartText<'_> {
    if part.kind != "text" {
        return PartText::NotText;
    }
    match part.fields.get("text") {
        None => PartText::Missing,
        Some(value) => value.as_str().map_or(PartText::NotString, PartText::Text),
    }
}

fn json_part(part: &Json) -> PartText<'_> {
    let Json::Object(fields) = part else {
        return PartText::NotText;
    };
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "text") {
        return PartText::NotText;
    }
    match fields.get("text") {
        None => PartText::Missing,
        Some(Json::Text(text)) => PartText::Text(text),
        Some(_) => PartText::NotString,
    }
}

pub fn message_text<T>(content: Option<&T>) -> Result<String, ClassifierTextError>
where
    T: ClassifierContent,
{
    let Some(content) = content else {
        return Ok(String::new());
    };
    match content.content_text() {
        ContentText::Plain(text) => Ok(text.to_owned()),
        ContentText::Parts(parts) => joined_text(parts),
    }
}

fn joined_text(parts: Vec<PartText<'_>>) -> Result<String, ClassifierTextError> {
    let mut texts: Vec<String> = Vec::with_capacity(parts.len());
    for (index, part) in parts.into_iter().enumerate() {
        match part {
            PartText::NotText => {}
            PartText::Missing => texts.push(String::new()),
            PartText::Text(text) => texts.push(text.to_owned()),
            PartText::NotString => {
                return Err(ClassifierTextError::PartTextNotString { part: index });
            }
        }
    }
    Ok(texts.join(" ").trim().to_owned())
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
        push_trimmed(&mut parts, &original, kept_from, start);
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
            .eq(needle.iter())
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
