pub mod content;
pub mod messages;
pub mod metrics;
pub mod timestamps;
pub mod trace;

mod support;
mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr as _;

// litellm.integrations.galileo.LLMResponse, the typed record a success callback appends to in_memory_records.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct LlmResponse {
    pub latency_ms: i64,
    pub status_code: i64,
    pub input_text: String,
    pub output_text: String,
    pub node_type: String,
    pub model: String,
    pub num_input_tokens: i64,
    pub num_output_tokens: i64,
    pub num_total_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_logprobs: Option<IndexMap<String, Json>>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_metadata: Option<IndexMap<String, Json>>,
}

impl LlmResponse {
    // record.get("messages") in _record_to_v2_span reads this record's messages entry below, not a field of LlmResponse itself.
    #[must_use]
    pub fn to_record(&self, extra_messages: Option<Json>) -> IndexMap<String, Json> {
        let mut record = IndexMap::new();
        for (key, value) in [
            ("latency_ms", self.latency_ms),
            ("status_code", self.status_code),
            ("num_input_tokens", self.num_input_tokens),
            ("num_output_tokens", self.num_output_tokens),
            ("num_total_tokens", self.num_total_tokens),
        ] {
            record.insert(
                key.to_owned(),
                Json::Number(serde_json::Number::from(value)),
            );
        }
        record.insert("input_text".to_owned(), Json::Text(self.input_text.clone()));
        record.insert(
            "output_text".to_owned(),
            Json::Text(self.output_text.clone()),
        );
        record.insert("node_type".to_owned(), Json::Text(self.node_type.clone()));
        record.insert("model".to_owned(), Json::Text(self.model.clone()));
        self.insert_optional_fields(&mut record);
        if let Some(messages) = extra_messages {
            record.insert("messages".to_owned(), messages);
        }
        record
    }

    fn insert_optional_fields(&self, record: &mut IndexMap<String, Json>) {
        if let Some(cost) = self.cost
            && let Ok(number) = serde_json::Number::from_str(&cost.to_string())
        {
            record.insert("cost".to_owned(), Json::Number(number));
        }
        if let Some(output_logprobs) = &self.output_logprobs {
            record.insert(
                "output_logprobs".to_owned(),
                Json::Object(output_logprobs.clone()),
            );
        }
        record.insert("created_at".to_owned(), Json::Text(self.created_at.clone()));
        if let Some(tags) = &self.tags {
            record.insert(
                "tags".to_owned(),
                Json::List(tags.iter().cloned().map(Json::Text).collect()),
            );
        }
        if let Some(user_metadata) = &self.user_metadata {
            record.insert(
                "user_metadata".to_owned(),
                Json::Object(user_metadata.clone()),
            );
        }
    }
}
