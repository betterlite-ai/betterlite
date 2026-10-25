use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    #[serde(default = "search_object")]
    pub object: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
    #[serde(skip)]
    #[serde(rename = "_hidden_params")]
    pub hidden_params: BTreeMap<String, Value>,
}

impl SearchResponse {
    #[must_use]
    pub fn with_results(results: Vec<SearchResult>) -> Self {
        Self {
            results,
            object: search_object(),
            extra: BTreeMap::new(),
            hidden_params: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn hidden_params(&self) -> &BTreeMap<String, Value> {
        &self.hidden_params
    }

    pub fn set_hidden_params(&mut self, hidden_params: BTreeMap<String, Value>) {
        self.hidden_params = hidden_params;
    }
}

fn search_object() -> String {
    "search".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchToolLiteLLMParams {
    pub search_provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<i64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_id: Option<String>,
    pub search_tool_name: String,
    pub litellm_params: SearchToolLiteLLMParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_info: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchToolInfoResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_id: Option<String>,
    pub search_tool_name: String,
    pub litellm_params: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_info: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_from_config: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSearchToolsResponse {
    pub search_tools: Vec<SearchToolInfoResponse>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
