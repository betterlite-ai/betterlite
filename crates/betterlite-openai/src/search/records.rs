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
