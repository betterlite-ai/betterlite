use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningEffort {
    #[serde(rename = "none")]
    Disabled,
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    Xhigh,
    #[serde(rename = "max")]
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ranker {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "default_2024_08_21")]
    Default20240821,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankingOptions {
    pub score_threshold: Number,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub ranker: Option<Option<Ranker>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSearchOptions {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub max_num_results: Option<Option<u32>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub ranking_options: Option<Option<RankingOptions>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub parameters: Option<Option<Map<String, Value>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub strict: Option<Option<bool>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantTool {
    CodeInterpreter,
    FileSearch {
        #[serde(default, with = "serde_with::rust::double_option")]
        file_search: Option<Option<FileSearchOptions>>,
    },
    Function {
        function: FunctionDefinition,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticChunking {
    pub chunk_overlap_tokens: u32,
    pub max_chunk_size_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChunkingStrategy {
    Auto,
    Static {
        #[serde(rename = "static")]
        settings: StaticChunking,
    },
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorStoreSpec {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub chunking_strategy: Option<Option<ChunkingStrategy>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub file_ids: Option<Option<Vec<String>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub metadata: Option<Option<BTreeMap<String, String>>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeInterpreterResources {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub file_ids: Option<Option<Vec<String>>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSearchResources {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub vector_store_ids: Option<Option<Vec<String>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub vector_stores: Option<Option<Vec<VectorStoreSpec>>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResources {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub code_interpreter: Option<Option<CodeInterpreterResources>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub file_search: Option<Option<FileSearchResources>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonSchemaFormat {
    pub name: String,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub schema: Option<Option<Map<String, Value>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub strict: Option<Option<bool>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TypedFormat {
    Text,
    JsonObject,
    JsonSchema { json_schema: JsonSchemaFormat },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoFormat {
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseFormat {
    Auto(AutoFormat),
    Typed(TypedFormat),
}

// A create key that arrived as null stays null in the request body; one the caller left out is skipped.
#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCreate {
    pub model: String,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub instructions: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub metadata: Option<Option<BTreeMap<String, String>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub name: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub reasoning_effort: Option<Option<ReasoningEffort>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub response_format: Option<Option<ResponseFormat>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub temperature: Option<Option<Number>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub tool_resources: Option<Option<ToolResources>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub tools: Option<Option<Vec<AssistantTool>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub top_p: Option<Option<Number>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantObject {
    pub id: String,
    pub created_at: i64,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub model: String,
    pub name: Option<String>,
    pub object: String,
    pub tools: Vec<AssistantTool>,
    pub response_format: Option<ResponseFormat>,
    pub temperature: Option<Number>,
    pub tool_resources: Option<ToolResources>,
    pub top_p: Option<Number>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantPage {
    pub object: String,
    pub data: Vec<AssistantObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantDeleted {
    pub id: String,
    pub deleted: bool,
    pub object: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListParams {
    pub order: Option<String>,
    pub limit: Option<i64>,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Default for ListParams {
    fn default() -> Self {
        Self {
            order: Some(String::from("desc")),
            limit: Some(20),
            before: None,
            after: None,
        }
    }
}

impl ListParams {
    // The SDK sends the query in key order and leaves out a None or empty value, so only those reach the wire.
    #[must_use]
    pub fn request_params(&self) -> Vec<(String, String)> {
        let mut request_params = Vec::new();
        if let Some(after) = self.after.as_ref().filter(|after| !after.is_empty()) {
            request_params.push((String::from("after"), after.clone()));
        }
        if let Some(before) = self.before.as_ref().filter(|before| !before.is_empty()) {
            request_params.push((String::from("before"), before.clone()));
        }
        if let Some(limit) = self.limit {
            request_params.push((String::from("limit"), limit.to_string()));
        }
        if let Some(order) = self.order.as_ref().filter(|order| !order.is_empty()) {
            request_params.push((String::from("order"), order.clone()));
        }
        request_params
    }
}
