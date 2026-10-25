use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::openai_base_config::Object;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub last_updated: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SearchObject(pub String);

impl Default for SearchObject {
    fn default() -> Self {
        Self(String::from("search"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    #[serde(default)]
    pub object: SearchObject,
    #[serde(flatten)]
    pub extra: Object,
    // LiteLLM keeps these in a pydantic PrivateAttr, so they never reach the client
    #[serde(skip)]
    pub hidden_params: Object,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchToolLiteLLMParams {
    pub search_provider: String,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub timeout: Option<Decimal>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchTool {
    #[serde(default)]
    pub search_tool_id: Option<String>,
    pub search_tool_name: String,
    pub litellm_params: SearchToolLiteLLMParams,
    #[serde(default)]
    pub search_tool_info: Option<Object>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchToolInfoResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_id: Option<String>,
    pub search_tool_name: String,
    pub litellm_params: Object,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_tool_info: Option<Object>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_from_config: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSearchToolsResponse {
    pub search_tools: Vec<SearchToolInfoResponse>,
}
