use std::fmt;

use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::openai_base_config::Object;

#[derive(Clone, Deserialize)]
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
impl fmt::Debug for SearchToolLiteLLMParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchToolLiteLLMParams")
            .field("search_provider", &self.search_provider)
            .field("api_key", &self.api_key)
            .field("api_base", &self.api_base)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .field("extra_keys", &self.extra.keys().collect::<Vec<_>>())
            .finish()
    }
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
