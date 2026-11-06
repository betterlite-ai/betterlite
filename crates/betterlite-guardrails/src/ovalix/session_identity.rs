use std::collections::BTreeMap;

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub trait Clock: Send + Sync {
    fn today(&self) -> NaiveDate;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn today(&self) -> NaiveDate {
        Utc::now().date_naive()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestData {
    pub metadata: Option<BTreeMap<String, Option<String>>>,
    pub litellm_metadata: Option<BTreeMap<String, Option<String>>>,
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
            values
                .get("user_api_key_user_email")
                .and_then(|value| value.as_deref())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    values
                        .get("user_api_key_user_id")
                        .and_then(|value| value.as_deref())
                        .filter(|value| !value.is_empty())
                })
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
