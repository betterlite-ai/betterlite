use std::collections::BTreeMap;

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::value::RawValue;

const HEADER_NAME: &str = "anthropic-beta";

// An element of a client-supplied anthropic-beta list, kept as the JSON value the caller sent.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BetaValue {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

// A header value as LiteLLM reads it: text and lists are used, every other type counts as no value.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BetaHeader {
    Text(String),
    List(Vec<BetaValue>),
    Other(IgnoredAny),
}

// str.strip() also drops the four ASCII separator controls, which char::is_whitespace does not.
const fn is_space(ch: char) -> bool {
    match ch {
        '\u{1c}'..='\u{1f}' => true,
        _ => ch.is_whitespace(),
    }
}

// Python str() of a decoded JSON element; numbers and containers keep their JSON text.
fn element_text(element: &RawValue) -> String {
    let raw = element.get();
    match raw {
        "null" => "None".to_owned(),
        "true" => "True".to_owned(),
        "false" => "False".to_owned(),
        _ => serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_owned()),
    }
}

// Mirrors anthropic_beta_from_headers: a list as given, a JSON array or comma list as stripped strings.
#[must_use]
pub fn anthropic_beta_from_headers(headers: &BTreeMap<String, BetaHeader>) -> Vec<BetaValue> {
    let text = match headers.get(HEADER_NAME) {
        Some(BetaHeader::List(items)) => return items.clone(),
        Some(BetaHeader::Text(text)) if !text.is_empty() => text.trim_matches(is_space),
        Some(BetaHeader::Text(_) | BetaHeader::Other(_)) | None => return Vec::new(),
    };
    if text.starts_with('[')
        && text.ends_with(']')
        && let Ok(items) = serde_json::from_str::<Vec<Box<RawValue>>>(text)
    {
        return items
            .iter()
            .map(|item| BetaValue::Text(element_text(item).trim_matches(is_space).to_owned()))
            .collect();
    }
    text.split(',')
        .map(|beta| BetaValue::Text(beta.trim_matches(is_space).to_owned()))
        .collect()
}
