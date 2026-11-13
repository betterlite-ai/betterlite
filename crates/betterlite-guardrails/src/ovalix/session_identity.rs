use std::collections::BTreeMap;

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::primitives::json_value::JsonValue;

pub trait Clock: Send + Sync {
    fn today(&self) -> NaiveDate;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn today(&self) -> NaiveDate {
        Utc::now().date_naive()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestData {
    pub metadata: Option<BTreeMap<String, JsonValue>>,
    pub litellm_metadata: Option<BTreeMap<String, JsonValue>>,
}

// only a string value names an actor; LiteLLM reads the dict the same way and skips anything else
fn text_value<'values>(
    values: &'values BTreeMap<String, JsonValue>,
    key: &str,
) -> Option<&'values str> {
    match values.get(key) {
        Some(JsonValue::String(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

// LiteLLM's `metadata or litellm_metadata` skips an empty dict, so a populated litellm_metadata still wins
#[must_use]
pub fn actor(data: &RequestData) -> &str {
    let metadata = data.metadata.as_ref().filter(|values| !values.is_empty());
    let litellm_metadata = data
        .litellm_metadata
        .as_ref()
        .filter(|values| !values.is_empty());
    metadata
        .or(litellm_metadata)
        .and_then(|values| {
            text_value(values, "user_api_key_user_email")
                .or_else(|| text_value(values, "user_api_key_user_id"))
        })
        .unwrap_or("unknown")
}

#[must_use]
pub fn tracker_actor_id(data: &RequestData) -> String {
    hex::encode(Sha256::digest(actor(data).as_bytes()))
        .chars()
        .take(8)
        .collect()
}

#[must_use]
pub fn session_id(data: &RequestData, today: NaiveDate, application_id: &str) -> String {
    format!(
        "{}_{}_{}",
        tracker_actor_id(data),
        today.format("%Y-%m-%d"),
        application_id
    )
}

mod tests;
