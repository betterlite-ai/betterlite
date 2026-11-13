use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::json::Json;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Tool {
    Function(Function),
    GoogleSearch,
    CodeExecution,
    UrlContext,
    ComputerUse(ComputerUse),
    McpServer(McpServer),
    FileSearch(FileSearch),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Function {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameters: Option<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ComputerUse {
    pub environment: Option<Environment>,
    // the OpenAPI schema spells this field in camel case
    #[serde(default, rename = "excludedPredefinedFunctions")]
    pub excluded_predefined_functions: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Browser,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct McpServer {
    pub name: Option<String>,
    pub url: Option<String>,
    pub headers: Option<IndexMap<String, String>>,
    pub allowed_tools: Option<Vec<AllowedTools>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AllowedTools {
    pub mode: Option<ToolChoiceType>,
    pub tools: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoiceType {
    Auto,
    Any,
    None,
    Validated,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(ToolChoiceType),
    Config(ToolChoiceConfig),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolChoiceConfig {
    pub allowed_tools: Option<AllowedTools>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileSearch {
    pub file_search_store_names: Option<Vec<String>>,
    pub top_k: Option<i64>,
    pub metadata_filter: Option<String>,
}
