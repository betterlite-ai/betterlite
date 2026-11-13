use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type WireRecord = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VectorStoreToolParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_num_results: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_options: Option<Value>,
}

impl VectorStoreToolParams {
    #[must_use]
    pub fn to_dict(&self) -> WireRecord {
        let mut result = WireRecord::new();
        if let Some(value) = self.filters.as_ref().filter(|value| !value.is_null()) {
            result.insert("filters".to_owned(), value.clone());
        }
        if let Some(value) = self.max_num_results {
            result.insert("max_num_results".to_owned(), Value::from(value));
        }
        if let Some(value) = self
            .ranking_options
            .as_ref()
            .filter(|value| !value.is_null())
        {
            result.insert("ranking_options".to_owned(), value.clone());
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreIntegration {
    Bedrock,
    Ragflow,
}

pub type SupportedIntegrations = VectorStoreIntegration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreOpenAiParam {
    Filters,
    MaxNumResults,
    RankingOptions,
    RewriteQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreIndexType {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreSearchFailureMode {
    Annotate,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorStoreStatus {
    Expired,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuardrailAction {
    Intervened,
    None,
}
