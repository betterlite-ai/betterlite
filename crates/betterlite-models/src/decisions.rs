pub mod ir;
pub mod openai;
pub mod systemone;
pub mod text;
pub mod validators;

use chrono::{DateTime, NaiveDateTime, TimeZone as _, Utc};
use indexmap::IndexMap;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub const MAX_DECISION_QUESTIONS: usize = 128;

pub type Extra = IndexMap<String, Json>;
pub type HiddenParams = IndexMap<String, Json>;

// numbers stay serde_json's Number because the workspace's arbitrary_precision breaks u64 and f64 under flatten
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct TokenCount(pub Number);

impl Default for TokenCount {
    fn default() -> Self {
        Self(Number::from(0_u8))
    }
}

// serde_json's Value sorts object keys unless a workspace-wide feature is on, and LiteLLM keeps them as sent
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Json {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DecisionsJson {
    Text(String),
    Object(IndexMap<String, Json>),
    List(Vec<Json>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DateTimeInput {
    Text(String),
    UnixTimestamp(i64),
}

pub(crate) fn deserialize_optional_datetime<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<DateTimeInput>::deserialize(deserializer)?;
    value
        .map(|input| match input {
            DateTimeInput::Text(text) => DateTime::parse_from_rfc3339(&text)
                .map(|datetime| datetime.with_timezone(&Utc))
                .or_else(|_| {
                    NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S%.f")
                        .map(|datetime| DateTime::from_naive_utc_and_offset(datetime, Utc))
                })
                .map_err(Error::custom),
            DateTimeInput::UnixTimestamp(timestamp) => Utc
                .timestamp_opt(timestamp, 0)
                .single()
                .ok_or_else(|| Error::custom("invalid unix timestamp")),
        })
        .transpose()
}
