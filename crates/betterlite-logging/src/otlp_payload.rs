use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::otlp_normalize::{OTLP_MAX_ATTRIBUTE_VALUE_BYTES, truncate};
use crate::python_json::{dumps, dumps_object};

mod tests;

type Message = IndexMap<String, Json>;

// A character json.dumps escapes as \uXXXX takes six bytes
const MAX_JSON_ESCAPE_BYTES: usize = 6;

#[must_use]
pub fn truncate_payload(value: &str) -> String {
    if value.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES || !value.starts_with('[') {
        return truncate(value);
    }
    let Ok(messages) = serde_json::from_str::<Vec<Message>>(value) else {
        return truncate(value);
    };
    let [first, .., last] = messages.as_slice() else {
        return truncate(value);
    };
    let count = messages.len();
    let head = dumps_object(first);
    let rest: Vec<String> = messages.iter().skip(1).map(dumps_object).collect();
    let marker_budget = elided(count).len().saturating_add(1);
    let budget = OTLP_MAX_ATTRIBUTE_VALUE_BYTES
        .saturating_sub(3)
        .saturating_sub(head.len())
        .saturating_sub(marker_budget);
    let kept = newest_that_fit(&rest, budget).min(count.saturating_sub(2));
    if kept == 0 {
        return shortened(first, last, count, marker_budget);
    }
    let marker = elided(count.saturating_sub(1).saturating_sub(kept));
    let tail = rest.iter().skip(rest.len().saturating_sub(kept));
    let parts: Vec<&str> = [head.as_str(), marker.as_str()]
        .into_iter()
        .chain(tail.map(String::as_str))
        .collect();
    format!("[{}]", parts.join(", "))
}

// Not even one trailing message fits beside the first, so both ends are cut down to half the limit each
fn shortened(first: &Message, last: &Message, count: usize, marker_budget: usize) -> String {
    let half = OTLP_MAX_ATTRIBUTE_VALUE_BYTES
        .saturating_sub(marker_budget)
        .saturating_sub(4)
        .div_euclid(2);
    let middle = (count > 2).then(|| elided(count.saturating_sub(2)));
    let parts: Vec<String> = [
        Some(shrunk_message(first, half)),
        middle,
        Some(shrunk_message(last, half)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let shrunk = format!("[{}]", parts.join(", "));
    if shrunk.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES {
        return shrunk;
    }
    format!("[{}]", elided(count))
}

fn elided(count: usize) -> String {
    let content = format!("\u{2026}[{count} earlier messages truncated]");
    let marker = IndexMap::from([
        ("role".to_owned(), Json::Text("system".to_owned())),
        ("content".to_owned(), Json::Text(content)),
    ]);
    dumps_object(&marker)
}

// How many trailing messages fit in the budget with a separator byte each, counted newest first
fn newest_that_fit(encoded: &[String], budget: usize) -> usize {
    let mut total = 0_usize;
    for (count, message) in encoded.iter().rev().enumerate() {
        total = total.saturating_add(message.len()).saturating_add(1);
        if total > budget {
            return count;
        }
    }
    encoded.len()
}

// Shortens the content first, then drops every other field, then assumes each kept byte escapes to six
fn shrunk_message(message: &Message, budget: usize) -> String {
    let text = if let Some(Json::Text(text)) = message.get("content") {
        text.clone()
    } else {
        dumps(message.get("content").unwrap_or(&Json::Null))
    };
    let role = message
        .get("role")
        .cloned()
        .unwrap_or_else(|| Json::Text("user".to_owned()));
    let role_only = IndexMap::from([("role".to_owned(), role)]);
    let escaped = cut_content(&role_only, &text, budget, MAX_JSON_ESCAPE_BYTES);
    [
        cut_content(message, &text, budget, 1),
        cut_content(&role_only, &text, budget, 1),
    ]
    .into_iter()
    .find(|attempt| attempt.len() <= budget)
    .unwrap_or(escaped)
}

fn cut_content(message: &Message, text: &str, budget: usize, escape_factor: usize) -> String {
    let overhead = with_content(message, "").len();
    let room = budget
        .saturating_sub(overhead)
        .saturating_sub(48)
        .checked_div(escape_factor)
        .unwrap_or_default();
    let kept = text
        .get(..text.floor_char_boundary(room))
        .unwrap_or_default();
    let marked = format!(
        "{kept}\u{2026}[truncated {} bytes]",
        text.len().saturating_sub(kept.len())
    );
    with_content(message, &marked)
}

// Replacing content keeps its place among the keys, and a message without one gets it last
fn with_content(message: &Message, content: &str) -> String {
    let mut replaced = message.clone();
    replaced.insert("content".to_owned(), Json::Text(content.to_owned()));
    dumps_object(&replaced)
}
